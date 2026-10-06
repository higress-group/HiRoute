//! ACP v1 only. No reconnect, new-session fallback, file/terminal RPC, or persistent approval.
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AuthenticateRequest, CancelNotification, ClientCapabilities, InitializeRequest,
    LoadSessionRequest, NewSessionRequest, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionNotification, SetSessionModeRequest,
};
use agent_client_protocol::{Agent, ConnectionTo};
use hiroute_domain::delegation::DelegationErrorV1;
use serde_json::{Map, Value, json};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{Instant, timeout, timeout_at};
use tokio_util::sync::CancellationToken;

use super::progress::ProgressSink;

mod identity;
mod model;
mod pi_failure;
mod transport;
pub use identity::AcpNativeIdentityContract;
pub use transport::MAX_ACP_FRAME_BYTES;

pub use hiroute_domain::delegation::DelegationSessionBindingV1 as AcpSessionBinding;

#[derive(Clone, Debug)]
pub enum AcpSessionStart {
    New,
    Load(AcpSessionBinding),
    Resume(AcpSessionBinding),
}

/// Internal profile output, never a raw Local Control payload. Authentication can contain
/// a run secret: deliberately no Debug/Serialize implementation and no protocol logging.
pub struct AcpRunInput {
    pub cwd: PathBuf,
    pub prompt: String,
    pub session: AcpSessionStart,
    pub identity_contract: AcpNativeIdentityContract,
    pub session_meta: Map<String, Value>,
    /// Exact adapter-owned native mode selected after new/load and before the first prompt.
    /// Production profiles always set this; `None` exists for protocol-only test fixtures.
    pub native_session_mode: Option<String>,
    /// Profile's exact native model selector ID, confirmed before prompting. The frozen
    /// Plan/Gateway alias remains separate when the native client adds a provider prefix.
    /// Production runs and installation probes set this; protocol-only fixtures may omit it.
    pub expected_model: Option<String>,
    pub authentication: Option<Value>,
    pub deadline: Instant,
    pub cancellation: CancellationToken,
}

pub trait AcpRunJournal: Send + Sync {
    /// Transient startup receipt after a successful, version-checked initialize.
    /// It does not imply session/model readiness or permission to send a prompt.
    fn initialized(&self) {}
    fn session_bound(&self, binding: &AcpSessionBinding) -> Result<(), DelegationErrorV1>;
    /// Must durably record the send intent before returning Ok. Failure means no prompt.
    fn before_prompt(&self) -> Result<(), DelegationErrorV1>;
    fn text_update(&self, text: &str) -> Result<(), DelegationErrorV1>;
    /// Compatibility fallback for an unexpected permission request. Recheck the current run
    /// lifecycle and selected policy on EACH request; this is not the normal autonomy mechanism.
    fn allow_permission_once(&self, request: &Value) -> bool;
}

pub struct AcpRunOutcome {
    pub session: AcpSessionBinding,
    pub stop_reason: String,
    pub text: String,
    pub content_incomplete: bool,
}

#[derive(Default)]
struct Updates {
    session: Option<String>,
    text: String,
    incomplete: bool,
    prompt_started: bool,
}

pub async fn run_acp<R, W>(
    reader: R,
    writer: W,
    input: AcpRunInput,
    journal: Arc<dyn AcpRunJournal>,
) -> Result<AcpRunOutcome, DelegationErrorV1>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    run_acp_with_progress(reader, writer, input, journal, None).await
}

pub(crate) async fn run_acp_with_progress<R, W>(
    reader: R,
    writer: W,
    input: AcpRunInput,
    journal: Arc<dyn AcpRunJournal>,
    progress: Option<ProgressSink>,
) -> Result<AcpRunOutcome, DelegationErrorV1>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    if !input.cwd.is_absolute() || input.prompt.is_empty() || input.prompt.len() > 256 * 1024 {
        return Err(DelegationErrorV1::InvalidArguments);
    }
    let updates = Arc::new(Mutex::new(Updates::default()));
    let notifications = updates.clone();
    let notification_journal = journal.clone();
    let notification_progress = progress;
    let permissions = updates.clone();
    let permission_journal = journal.clone();
    let permission_cancellation = input.cancellation.clone();
    let connection_cancellation = input.cancellation.clone();
    let permission_deadline = input.deadline;
    let hard_deadline = input.deadline + Duration::from_secs(5);
    let connected = agent_client_protocol::Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                let mut state = notifications.lock().unwrap_or_else(|e| e.into_inner());
                let session_matches =
                    state.session.as_deref() == Some(notification.session_id.to_string().as_str());
                let update = serde_json::to_value(notification.update).unwrap_or(Value::Null);
                if !session_matches {
                    state.incomplete = true;
                    return Ok(());
                }
                // session/load may replay old history: never append it as this run's output.
                if !state.prompt_started {
                    return Ok(());
                }
                if update["sessionUpdate"] == "agent_message_chunk"
                    && let Some(text) = update.pointer("/content/text").and_then(Value::as_str)
                {
                    if let Some(progress) = notification_progress.as_ref() {
                        progress.push(text);
                    }
                    if state.text.len().saturating_add(text.len()) <= MAX_ACP_FRAME_BYTES {
                        state.text.push_str(text);
                        if notification_journal.text_update(text).is_err() {
                            state.incomplete = true;
                        }
                    } else {
                        state.incomplete = true;
                    }
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                let matches = {
                    let state = permissions.lock().unwrap_or_else(|e| e.into_inner());
                    state.prompt_started
                        && state.session.as_deref() == Some(request.session_id.to_string().as_str())
                };
                let request_value = serde_json::to_value(&request).unwrap_or(Value::Null);
                let selected = (matches
                    && !permission_cancellation.is_cancelled()
                    && Instant::now() < permission_deadline)
                    .then(|| {
                        if permission_journal.allow_permission_once(&request_value) {
                            request
                                .options
                                .iter()
                                .find(|o| o.kind == PermissionOptionKind::AllowOnce)
                        } else {
                            None
                        }
                    })
                    .flatten();
                let outcome = selected.map_or(RequestPermissionOutcome::Cancelled, |option| {
                    RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                        option.option_id.clone(),
                    ))
                });
                responder.respond(RequestPermissionResponse::new(outcome))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(
            transport::bounded_transport(reader, writer),
            async move |connection: ConnectionTo<Agent>| {
                Ok(run_session(connection, input, journal, updates).await)
            },
        );
    tokio::pin!(connected);
    tokio::select! {
        biased;
        _ = connection_cancellation.cancelled() => {
            // Give the ACP notification/prompt response a bounded graceful window. Some adapters
            // keep their transport open after an ignored prompt cancellation; the lifecycle must
            // then return so its existing owned-process stop and material cleanup can run.
            match timeout(Duration::from_secs(6), &mut connected).await {
                Ok(Ok(Ok(outcome))) => Ok(outcome),
                _ => Err(DelegationErrorV1::Cancelled),
            }
        },
        result = timeout_at(hard_deadline, &mut connected) => {
            result
                .map_err(|_| DelegationErrorV1::DeadlineExceeded)?
                .map_err(|_| DelegationErrorV1::ProtocolFailed)?
        },
    }
}

async fn run_session(
    connection: ConnectionTo<Agent>,
    input: AcpRunInput,
    journal: Arc<dyn AcpRunJournal>,
    updates: Arc<Mutex<Updates>>,
) -> Result<AcpRunOutcome, DelegationErrorV1> {
    let initialized = phase(
        &input.cancellation,
        input.deadline,
        connection.send_request(initialize_request()?).block_task(),
    )
    .await?;
    if initialized.protocol_version != ProtocolVersion::V1 {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    journal.initialized();
    if let Some(authentication) = input.authentication {
        let request: AuthenticateRequest = serde_json::from_value(authentication)
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        phase(
            &input.cancellation,
            input.deadline,
            connection.send_request(request).block_task(),
        )
        .await?;
    }
    let (binding, modes, config_options) = match input.session {
        AcpSessionStart::New => {
            let request: NewSessionRequest = serde_json::from_value(json!({
                "cwd":input.cwd,"mcpServers":[],"_meta":input.session_meta,
            }))
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
            let response = phase(
                &input.cancellation,
                input.deadline,
                connection.send_request(request).block_task(),
            )
            .await?;
            let raw =
                serde_json::to_value(&response).map_err(|_| DelegationErrorV1::ProtocolFailed)?;
            (
                AcpSessionBinding {
                    acp_session_id: response.session_id.to_string(),
                    native_session_id: input
                        .identity_contract
                        .new_native(&response.session_id.to_string(), &raw)?,
                },
                response.modes,
                response.config_options,
            )
        }
        AcpSessionStart::Resume(expected) => {
            if initialized
                .agent_capabilities
                .session_capabilities
                .resume
                .is_none()
                || input.identity_contract != AcpNativeIdentityContract::DshSessionV1
                || expected.native_session_id.as_deref() != Some(expected.acp_session_id.as_str())
                || expected.acp_session_id.is_empty()
            {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            updates.lock().unwrap_or_else(|e| e.into_inner()).session =
                Some(expected.acp_session_id.clone());
            let request: ResumeSessionRequest = serde_json::from_value(json!({
                "sessionId":expected.acp_session_id,"cwd":input.cwd,"mcpServers":[],
            }))
            .map_err(|_| DelegationErrorV1::InvalidArguments)?;
            let response = phase(
                &input.cancellation,
                input.deadline,
                connection.send_request(request).block_task(),
            )
            .await
            .map_err(|error| {
                if error == DelegationErrorV1::ProtocolFailed {
                    DelegationErrorV1::ResumeUnavailable
                } else {
                    error
                }
            })?;
            (expected, response.modes, response.config_options)
        }
        AcpSessionStart::Load(expected) => {
            let prerequisites_ready = initialized.agent_capabilities.load_session
                && expected.native_session_id.is_some()
                && !expected.acp_session_id.is_empty()
                && input.identity_contract != AcpNativeIdentityContract::Unverified;
            if !prerequisites_ready {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            updates.lock().unwrap_or_else(|e| e.into_inner()).session =
                Some(expected.acp_session_id.clone());
            let request: LoadSessionRequest = serde_json::from_value(json!({
                "sessionId":expected.acp_session_id,"cwd":input.cwd,"mcpServers":[],"_meta":input.session_meta,
            })).map_err(|_| DelegationErrorV1::InvalidArguments)?;
            let response = phase(
                &input.cancellation,
                input.deadline,
                connection.send_request(request).block_task(),
            )
            .await
            .map_err(|error| {
                if error == DelegationErrorV1::ProtocolFailed {
                    DelegationErrorV1::ResumeUnavailable
                } else {
                    error
                }
            })?;
            let raw =
                serde_json::to_value(&response).map_err(|_| DelegationErrorV1::ProtocolFailed)?;
            let identity_verified = input.identity_contract.verify_load(&expected, &raw);
            if !identity_verified {
                return Err(DelegationErrorV1::ResumeUnavailable);
            }
            (expected, response.modes, response.config_options)
        }
    };
    if binding.acp_session_id.is_empty() || binding.acp_session_id.len() > 1024 {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    updates.lock().unwrap_or_else(|e| e.into_inner()).session =
        Some(binding.acp_session_id.clone());
    journal.session_bound(&binding)?;
    if let Some(expected_model) = input.expected_model.as_deref() {
        model::ensure_model(
            &connection,
            &binding.acp_session_id,
            config_options.as_deref(),
            expected_model,
            &input.cancellation,
            input.deadline,
        )
        .await?;
    }
    if let Some(native_session_mode) = input.native_session_mode {
        let mode_available = modes.as_ref().is_some_and(|modes| {
            modes
                .available_modes
                .iter()
                .any(|mode| mode.id.to_string() == native_session_mode)
        });
        if !mode_available {
            return Err(DelegationErrorV1::CapabilityUnavailable);
        }
        phase(
            &input.cancellation,
            input.deadline,
            connection
                .send_request(SetSessionModeRequest::new(
                    binding.acp_session_id.clone(),
                    native_session_mode,
                ))
                .block_task(),
        )
        .await
        .map_err(|error| {
            if error == DelegationErrorV1::ProtocolFailed {
                DelegationErrorV1::CapabilityUnavailable
            } else {
                error
            }
        })?;
    }
    if input.cancellation.is_cancelled() {
        return Err(DelegationErrorV1::Cancelled);
    }
    if Instant::now() >= input.deadline {
        return Err(DelegationErrorV1::DeadlineExceeded);
    }
    let request: PromptRequest = serde_json::from_value(json!({
        "sessionId":binding.acp_session_id,"prompt":[{"type":"text","text":input.prompt}],
    }))
    .map_err(|_| DelegationErrorV1::InvalidArguments)?;
    journal.before_prompt()?;
    updates
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .prompt_started = true;
    let prompt = connection.send_request(request).block_task();
    tokio::pin!(prompt);
    let response = tokio::select! {
        biased;
        _ = input.cancellation.cancelled() => None,
        _ = tokio::time::sleep_until(input.deadline) => None,
        response = &mut prompt => Some(response.map_err(pi_failure::map_error)?),
    };
    let stop_reason = if let Some(response) = response {
        let prompt_failed = prompt_session_failure(response.meta.as_ref())?;
        let stop_reason = serde_json::to_value(response.stop_reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or(DelegationErrorV1::ProtocolFailed)?;
        if prompt_failed {
            return Err(DelegationErrorV1::PromptFailed);
        }
        stop_reason
    } else {
        let cancel: CancelNotification =
            serde_json::from_value(json!({"sessionId":binding.acp_session_id}))
                .map_err(|_| DelegationErrorV1::InvalidArguments)?;
        connection
            .send_notification(cancel)
            .map_err(|_| DelegationErrorV1::ProtocolFailed)?;
        timeout(Duration::from_secs(5), &mut prompt)
            .await
            .map_err(|_| DelegationErrorV1::DeadlineExceeded)?
            .map_err(|_| DelegationErrorV1::ProtocolFailed)?;
        "cancelled".to_owned()
    };
    let mut state = updates.lock().unwrap_or_else(|e| e.into_inner());
    Ok(AcpRunOutcome {
        session: binding,
        stop_reason,
        text: std::mem::take(&mut state.text),
        content_incomplete: state.incomplete,
    })
}

fn initialize_request() -> Result<InitializeRequest, DelegationErrorV1> {
    let client_capabilities: ClientCapabilities = serde_json::from_value(json!({
        "_meta": {
            "jetbrains": {
                "air": {
                    "version": 1,
                    "capabilities": ["sessionFailure"]
                }
            }
        }
    }))
    .map_err(|_| DelegationErrorV1::ProtocolFailed)?;
    Ok(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(client_capabilities))
}

/// The negotiated v1 extension reports a terminal model/adapter error on a successful ACP
/// PromptResponse. Treating only the JSON-RPC envelope as authoritative would falsely complete
/// the Worker run. Unknown metadata outside this exact extension remains opaque.
fn prompt_session_failure(meta: Option<&Map<String, Value>>) -> Result<bool, DelegationErrorV1> {
    let Some(air) = meta
        .and_then(|meta| meta.get("jetbrains"))
        .and_then(Value::as_object)
        .and_then(|jetbrains| jetbrains.get("air"))
        .and_then(Value::as_object)
    else {
        return Ok(false);
    };
    let Some(failure) = air.get("sessionFailure") else {
        return Ok(false);
    };
    if !air
        .get("version")
        .and_then(Value::as_u64)
        .is_some_and(|version| version >= 1)
    {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    let failure = failure
        .as_object()
        .ok_or(DelegationErrorV1::ProtocolFailed)?;
    let required_string = |field: &str, limit: usize| {
        failure
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| {
                !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
            })
            .ok_or(DelegationErrorV1::ProtocolFailed)
    };
    required_string("id", 1_024)?;
    if !failure
        .get("title")
        .and_then(Value::as_str)
        .is_some_and(|title| !title.is_empty() && title.len() <= MAX_ACP_FRAME_BYTES)
    {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    if failure.get("revision").and_then(Value::as_u64) == Some(0)
        || failure.get("revision").and_then(Value::as_u64).is_none()
    {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    if !matches!(
        required_string("category", 32)?,
        "connection" | "access" | "limit" | "request" | "service" | "unknown"
    ) {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    let severity = required_string("severity", 16)?;
    if !matches!(severity, "warning" | "error") {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    if failure.get("details").is_some_and(|details| {
        !details
            .as_str()
            .is_some_and(|details| details.len() <= MAX_ACP_FRAME_BYTES)
    }) {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    let actions = failure
        .get("actions")
        .and_then(Value::as_array)
        .filter(|actions| actions.len() <= 32)
        .ok_or(DelegationErrorV1::ProtocolFailed)?;
    if actions.iter().any(|action| {
        !action.as_str().is_some_and(|action| {
            !action.is_empty() && action.len() <= 64 && !action.chars().any(char::is_control)
        })
    }) {
        return Err(DelegationErrorV1::ProtocolFailed);
    }
    Ok(severity == "error")
}

async fn phase<T>(
    cancellation: &CancellationToken,
    deadline: Instant,
    future: impl Future<Output = Result<T, agent_client_protocol::Error>>,
) -> Result<T, DelegationErrorV1> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(DelegationErrorV1::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(DelegationErrorV1::DeadlineExceeded),
        result = future => result.map_err(pi_failure::map_error),
    }
}

#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod model_tests;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
mod tests;
