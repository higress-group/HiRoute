//! OS-trusted credential minting. A collaboration grant can never enter this path.
use super::auth::{AuthError, ConsoleAuth, LaunchTarget};
use hiroute_application_api::*;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

pub trait ConsoleEndpoint: Send + Sync {
    fn launch_target(&self, loopback_only: bool) -> Result<LaunchTarget, AuthError>;

    fn qualifies_native_assets(&self, _origin: &str, _assets_sha256: &str) -> bool {
        false
    }
}
pub struct ConsoleControl {
    pub(super) auth: Arc<Mutex<ConsoleAuth>>,
    endpoint: OnceLock<Arc<dyn ConsoleEndpoint>>,
}
impl ConsoleControl {
    pub fn open(data_root: &Path) -> Result<Self, AuthError> {
        Ok(Self {
            auth: Arc::new(Mutex::new(ConsoleAuth::open(data_root)?)),
            endpoint: OnceLock::new(),
        })
    }
    pub fn set_endpoint(&self, endpoint: Arc<dyn ConsoleEndpoint>) -> Result<(), AuthError> {
        self.endpoint
            .set(endpoint)
            .map_err(|_| AuthError::Unavailable)
    }
    pub(crate) fn dispatch(
        &self,
        request: &LocalControlWireRequestV2,
    ) -> Option<MachineEnvelopeV2<Value>> {
        if !matches!(
            request.operation_id.as_str(),
            "CreateConsoleLaunch"
                | "CreateConsoleToken"
                | "ListConsoleTokens"
                | "RevokeConsoleToken"
                | QUALIFY_NATIVE_CONSOLE_V1
        ) {
            return None;
        }
        let result = if request.protected_grant.is_some() {
            Err(ErrorCode::CapabilityDenied)
        } else {
            self.execute(request).map_err(|e| match e {
                AuthError::Invalid => ErrorCode::InvalidArguments,
                AuthError::ListenerUnavailable => ErrorCode::ActionRequired,
                _ => ErrorCode::DaemonUnavailable,
            })
        };
        Some(match result {
            Ok(data) => MachineEnvelopeV2::succeeded(data, Some(request.request_id.clone())),
            Err(code) => {
                MachineEnvelopeV2::failed(ErrorV1::new(code), Some(request.request_id.clone()))
            }
        })
    }
    fn execute(&self, request: &LocalControlWireRequestV2) -> Result<Value, AuthError> {
        match request.operation_id.as_str() {
            QUALIFY_NATIVE_CONSOLE_V1 => {
                let input: QualifyNativeConsoleRequestV1 =
                    serde_json::from_value(request.payload.clone())
                        .map_err(|_| AuthError::Invalid)?;
                let qualified = self.endpoint.get().is_some_and(|endpoint| {
                    endpoint.qualifies_native_assets(&input.origin, &input.assets_sha256)
                });
                if !qualified {
                    return Ok(json!(false));
                }
                if let Some(session) = input.session {
                    return Ok(json!(
                        self.auth
                            .lock()
                            .map_err(|_| AuthError::Unavailable)?
                            .native_session_active(
                                &session.bearer_sha256,
                                &session.launch_sha256
                            )?
                    ));
                }
                Ok(json!(true))
            }
            "CreateConsoleLaunch" => {
                let input: ConsoleLaunchRequestV1 = serde_json::from_value(request.payload.clone())
                    .map_err(|_| AuthError::Invalid)?;
                let target = self
                    .endpoint
                    .get()
                    .ok_or(AuthError::Unavailable)?
                    .launch_target(input.loopback_only)?;
                let url = target.origin.clone();
                let code = self
                    .auth
                    .lock()
                    .map_err(|_| AuthError::Unavailable)?
                    .create_launch(target)?;
                Ok(json!({"url":format!("{url}/launch?code={}", code.as_str()),"expires_in":120}))
            }
            "CreateConsoleToken" => {
                let input: ConsoleCreateTokenRequestV1 =
                    serde_json::from_value(request.payload.clone())
                        .map_err(|_| AuthError::Invalid)?;
                let created = self
                    .auth
                    .lock()
                    .map_err(|_| AuthError::Unavailable)?
                    .create_token(input.name, input.expires_days)?;
                serde_json::to_value(created).map_err(|_| AuthError::Unavailable)
            }
            "ListConsoleTokens" => {
                let _: ConsoleListTokensRequestV1 = serde_json::from_value(request.payload.clone())
                    .map_err(|_| AuthError::Invalid)?;
                Ok(
                    json!({"tokens":self.auth.lock().map_err(|_|AuthError::Unavailable)?.list_tokens()}),
                )
            }
            "RevokeConsoleToken" => {
                let input: ConsoleRevokeTokenRequestV1 =
                    serde_json::from_value(request.payload.clone())
                        .map_err(|_| AuthError::Invalid)?;
                self.auth
                    .lock()
                    .map_err(|_| AuthError::Unavailable)?
                    .revoke_token(&input.id)?;
                Ok(json!({"revoked":true}))
            }
            _ => Err(AuthError::Invalid),
        }
    }
}
