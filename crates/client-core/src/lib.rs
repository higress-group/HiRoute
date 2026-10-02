#![forbid(unsafe_code)]
//! Shared transport and observation. All product decisions remain in the Application.
mod compute_management;
mod endpoint;
mod error;
#[cfg(unix)]
mod frame;
mod model_catalog;
mod model_connections;
mod observation;
mod observation_queries;
mod subscriptions;
mod worker;

pub use endpoint::LocalEndpoint;
pub use error::{ClientFailure, FailureCode, SubmissionState};
pub use observation::ObservationCursor;

use hiroute_application_api::{
    ClientHelloV1, ErrorCode, LOCAL_CONTROL_RELEASE_VERSION, LOCAL_CONTROL_SCHEMA_V2,
    LocalControlWireRequestV2, MACHINE_ENVELOPE_SCHEMA_V2, MachineEnvelopeV2, PrincipalKind,
    ServerHelloV1,
};
use hiroute_diagnostics::context::DiagnosticContext;
use hiroute_diagnostics::correlation::CorrelationDomain;
use hiroute_diagnostics::error::EventErrorCode;
use hiroute_diagnostics::event::{
    ControlCallEnd, ControlOperation, ControlStage, ControlStageKind, DiagnosticEvent,
    SubmissionState as DiagnosticSubmissionState,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::time::Duration;
use zeroize::Zeroize;

#[derive(Clone, Debug)]
pub struct Client {
    endpoint: LocalEndpoint,
    name: String,
    timeout: Duration,
    diagnostics: Option<DiagnosticContext>,
}

// Even error/cancellation paths wipe the private wire copy. Never derive Debug for this guard.
struct PrivateRequest(LocalControlWireRequestV2);
impl Drop for PrivateRequest {
    fn drop(&mut self) {
        if let Some(grant) = &mut self.0.protected_grant {
            grant.capability.zeroize();
        }
    }
}

impl Client {
    pub fn new(name: impl Into<String>, endpoint: LocalEndpoint) -> Self {
        Self {
            endpoint,
            name: name.into(),
            timeout: Duration::from_secs(30),
            diagnostics: None,
        }
    }
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    /// Attach the host's diagnostic context. The client only records bounded transport
    /// stages and one call outcome per call; no wire payload ever reaches diagnostics.
    pub fn with_diagnostics(mut self, context: DiagnosticContext) -> Self {
        self.diagnostics = Some(context);
        self
    }
    pub fn endpoint(&self) -> &LocalEndpoint {
        &self.endpoint
    }

    pub async fn query<P: Serialize, R: DeserializeOwned>(
        &self,
        operation: &str,
        request_id: &str,
        payload: &P,
    ) -> Result<MachineEnvelopeV2<R>, ClientFailure> {
        let request = LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: request_id.into(),
            operation_id: operation.into(),
            payload: serde_json::to_value(payload)
                .map_err(|_| ClientFailure::before_send(FailureCode::FrameInvalid))?,
            protected_grant: None,
        };
        self.call_typed(request).await
    }

    pub async fn call_typed<R: DeserializeOwned>(
        &self,
        request: LocalControlWireRequestV2,
    ) -> Result<MachineEnvelopeV2<R>, ClientFailure> {
        let result = self.call_wire(request).await?;
        let data = result
            .data
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| ClientFailure::at(FailureCode::FrameInvalid, true))?;
        Ok(MachineEnvelopeV2 {
            schema_version: result.schema_version,
            request_id: result.request_id,
            status: result.status,
            data,
            operation: result.operation,
            warnings: result.warnings,
            next_actions: result.next_actions,
            error: result.error,
        })
    }

    #[cfg(unix)]
    pub async fn call_wire(
        &self,
        request: LocalControlWireRequestV2,
    ) -> Result<MachineEnvelopeV2<Value>, ClientFailure> {
        use tokio::io::{AsyncWriteExt, BufReader};
        let started = std::time::Instant::now();
        // One context per call: a busy call cannot spend another call's Debug budget.
        let context = self.diagnostics.as_ref().map(|context| context.call_span());
        let operation_kind = ControlOperation::from_wire(&request.operation_id);
        let request_token = self.diagnostics.as_ref().and_then(|context| {
            context.token(CorrelationDomain::ControlRequest, &request.request_id)
        });
        let stage = |kind: ControlStageKind, ok: bool, sent: bool, elapsed: std::time::Duration| {
            if let Some(context) = &context {
                context.emit(DiagnosticEvent::ControlStage(ControlStage {
                    stage: kind,
                    ok,
                    elapsed_ms: elapsed.as_millis() as u64,
                    sent: Some(sent),
                }));
            }
        };
        let request = PrivateRequest(request);
        let mut sent = false;
        let deadline = tokio::time::Instant::now()
            .checked_add(self.timeout)
            .ok_or_else(|| ClientFailure::before_send(FailureCode::Deadline))?;
        let operation = async {
            let step = std::time::Instant::now();
            let validated = self.endpoint.validate_path();
            stage(
                ControlStageKind::EndpointValidate,
                validated.is_ok(),
                sent,
                step.elapsed(),
            );
            validated?;
            let step = std::time::Instant::now();
            let peer = async {
                if request
                    .0
                    .protected_grant
                    .as_ref()
                    .is_some_and(|g| g.principal_kind == PrincipalKind::Desktop)
                    && self.endpoint.expected_pid.is_none()
                {
                    return Err(FailureCode::PeerRejected);
                }
                let stream = tokio::net::UnixStream::connect(self.endpoint.path())
                    .await
                    .map_err(|_| FailureCode::TransportUnavailable)?;
                self.endpoint.validate_peer(&stream)?;
                Ok::<_, FailureCode>(stream)
            };
            let stream = peer.await;
            stage(
                ControlStageKind::PeerVerify,
                stream.is_ok(),
                sent,
                step.elapsed(),
            );
            let stream = stream?;
            let mut stream = BufReader::new(stream);
            let step = std::time::Instant::now();
            let hello = ClientHelloV1 {
                api_version: LOCAL_CONTROL_SCHEMA_V2,
                machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
                client_name: self.name.clone(),
                client_version: LOCAL_CONTROL_RELEASE_VERSION.into(),
            };
            let handshake = async {
                stream
                    .get_mut()
                    .write_all(&frame::encode(&hello)?)
                    .await
                    .map_err(|_| FailureCode::TransportUnavailable)?;
                let first = frame::read(&mut stream).await?;
                match serde_json::from_slice::<ServerHelloV1>(&first) {
                    Ok(hello)
                        if hello.api_version == LOCAL_CONTROL_SCHEMA_V2
                            && hello.machine_schema_version == MACHINE_ENVELOPE_SCHEMA_V2
                            && hello.daemon_version == LOCAL_CONTROL_RELEASE_VERSION
                            && hello.release_version == LOCAL_CONTROL_RELEASE_VERSION
                            && hello.capabilities.iter().any(|c| c == "local-control-v2")
                            && (!matches!(
                                request.0.operation_id.as_str(),
                                "GetClientServiceStatus"
                                    | "ListAgentPlanCatalog"
                                    | "GetAgentPlanStatus"
                                    | "FindOperationByIdempotency"
                            ) || hello
                                .capabilities
                                .iter()
                                .any(|c| c == "client-access-v1")) => {}
                    Ok(_) => return Err(FailureCode::SchemaIncompatible),
                    Err(_) => {
                        let rejection: MachineEnvelopeV2<Value> = serde_json::from_slice(&first)
                            .map_err(|_| FailureCode::FrameInvalid)?;
                        if rejection.schema_version != MACHINE_ENVELOPE_SCHEMA_V2
                            || rejection.data.is_some()
                            || rejection.operation.is_some()
                            || rejection.error.as_ref().is_none_or(|e| {
                                !matches!(
                                    e.code,
                                    ErrorCode::SchemaIncompatible
                                        | ErrorCode::MachineSchemaIncompatible
                                        | ErrorCode::CapabilityDenied
                                        | ErrorCode::InvalidArguments
                                )
                            })
                            || rejection
                                .error
                                .as_ref()
                                .is_some_and(|e| rejection.status != e.code.status())
                        {
                            return Err(FailureCode::FrameInvalid);
                        }
                        return Ok(Some(rejection));
                    }
                }
                if !stream.buffer().is_empty() {
                    return Err(FailureCode::FrameInvalid);
                }
                Ok(None)
            };
            let handshake: Result<Option<MachineEnvelopeV2<Value>>, FailureCode> = handshake.await;
            stage(
                ControlStageKind::Hello,
                handshake.is_ok(),
                sent,
                step.elapsed(),
            );
            if let Some(rejection) = handshake? {
                return Ok(rejection);
            }
            let step = std::time::Instant::now();
            let frame = match frame::encode(&request.0) {
                Ok(frame) => frame,
                Err(code) => {
                    stage(ControlStageKind::RequestWrite, false, sent, step.elapsed());
                    return Err(code);
                }
            };
            sent = true; // Once a write starts, failure does not establish whether admission happened.
            let written = stream.get_mut().write_all(&frame).await;
            stage(
                ControlStageKind::RequestWrite,
                written.is_ok(),
                sent,
                step.elapsed(),
            );
            written.map_err(|_| FailureCode::TransportUnavailable)?;
            let step = std::time::Instant::now();
            let read = async {
                let bytes = frame::read(&mut stream).await?;
                if !stream.buffer().is_empty() {
                    return Err(FailureCode::FrameInvalid);
                }
                let result: MachineEnvelopeV2<Value> =
                    serde_json::from_slice(&bytes).map_err(|_| FailureCode::FrameInvalid)?;
                if result.schema_version != MACHINE_ENVELOPE_SCHEMA_V2 {
                    return Err(FailureCode::SchemaIncompatible);
                }
                if result.request_id.as_deref() != Some(&request.0.request_id) {
                    return Err(FailureCode::ResponseMismatch);
                }
                Ok(result)
            };
            let result = read.await;
            stage(
                ControlStageKind::ResponseRead,
                result.is_ok(),
                sent,
                step.elapsed(),
            );
            result
        };
        match tokio::time::timeout_at(deadline, operation).await {
            Ok(Ok(result)) => {
                emit_call_end(
                    &context,
                    operation_kind,
                    request_token,
                    DiagnosticSubmissionState::Sent,
                    result
                        .error
                        .as_ref()
                        .map(|error| category_error_code(error.category)),
                    started.elapsed(),
                );
                Ok(result)
            }
            Ok(Err(code)) => {
                let failure = ClientFailure::at(code, sent);
                emit_call_end(
                    &context,
                    operation_kind,
                    request_token,
                    submission_state(failure),
                    Some(event_error(code)),
                    started.elapsed(),
                );
                Err(failure)
            }
            Err(_) => {
                let failure = ClientFailure::at(FailureCode::Deadline, sent);
                emit_call_end(
                    &context,
                    operation_kind,
                    request_token,
                    submission_state(failure),
                    Some(EventErrorCode::Deadline),
                    started.elapsed(),
                );
                Err(failure)
            }
        }
    }

    #[cfg(not(unix))]
    pub async fn call_wire(
        &self,
        request: LocalControlWireRequestV2,
    ) -> Result<MachineEnvelopeV2<Value>, ClientFailure> {
        let _request = PrivateRequest(request);
        Err(ClientFailure::before_send(FailureCode::UnsupportedPlatform))
    }
}

/// Map a local client failure onto the closed event vocabulary.
impl From<FailureCode> for EventErrorCode {
    fn from(code: FailureCode) -> Self {
        event_error(code)
    }
}

/// Map a product error category onto the closed event vocabulary. A free function because
/// both enums are foreign to this crate.
pub fn category_error_code(category: hiroute_application_api::ErrorCategory) -> EventErrorCode {
    use hiroute_application_api::ErrorCategory as C;
    match category {
        C::Usage => EventErrorCode::Usage,
        C::Conflict => EventErrorCode::Conflict,
        C::Authorization => EventErrorCode::Authorization,
        C::NotFound => EventErrorCode::NotFound,
        C::Unavailable => EventErrorCode::Unavailable,
        C::ActionRequired => EventErrorCode::ActionRequired,
        C::Recovery => EventErrorCode::Recovery,
        C::Internal => EventErrorCode::Internal,
    }
}

fn event_error(code: FailureCode) -> EventErrorCode {
    match code {
        FailureCode::LocatorUnavailable => EventErrorCode::LocatorUnavailable,
        FailureCode::TransportUnavailable => EventErrorCode::TransportUnavailable,
        FailureCode::Deadline => EventErrorCode::Deadline,
        FailureCode::PeerRejected => EventErrorCode::PeerRejected,
        FailureCode::SchemaIncompatible => EventErrorCode::SchemaIncompatible,
        FailureCode::FrameInvalid => EventErrorCode::FrameInvalid,
        FailureCode::FrameTooLarge => EventErrorCode::FrameTooLarge,
        FailureCode::ResponseMismatch => EventErrorCode::ResponseMismatch,
        FailureCode::ProtectedInputUnavailable => EventErrorCode::ProtectedInputUnavailable,
        FailureCode::ObservationStopped => EventErrorCode::ObservationStopped,
        FailureCode::UnsupportedPlatform => EventErrorCode::UnsupportedPlatform,
    }
}

/// A failure after a write started cannot establish whether admission happened, so it is
/// reported as unknown instead of claiming the request never reached the daemon.
fn submission_state(failure: ClientFailure) -> DiagnosticSubmissionState {
    match failure.submission {
        SubmissionState::NotSent => DiagnosticSubmissionState::NotSent,
        SubmissionState::MayHaveReachedServer => DiagnosticSubmissionState::Unknown,
    }
}

fn emit_call_end(
    context: &Option<DiagnosticContext>,
    operation: ControlOperation,
    request_token: Option<hiroute_diagnostics::identity::CorrelationToken>,
    submission: DiagnosticSubmissionState,
    error: Option<EventErrorCode>,
    elapsed: Duration,
) {
    let Some(context) = context else {
        return;
    };
    context.emit(DiagnosticEvent::ControlCallEnd(ControlCallEnd {
        operation,
        request_token,
        submission,
        error,
        elapsed_ms: elapsed.as_millis() as u64,
    }));
}
