//! Authenticated v8 OAuth control. Only explicit safe projections leave this boundary.
use std::path::PathBuf;

use reqwest::Url;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::http::{LoopbackRequest, percent_encode_query, request};
use crate::managed_oauth::ManagedOAuthCredentialSource;
use crate::{CpaAccountKind, CpaLifecycleError, CpaManagedCredentialSummary};

use super::{ManagedCpaRuntime, RuntimeInner};

#[derive(Clone, Eq, PartialEq)]
pub struct CpaOAuthLogin {
    pub authorization_url: String,
    pub state: String,
}

impl std::fmt::Debug for CpaOAuthLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CpaOAuthLogin([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpaOAuthStatus {
    Pending,
    Complete,
    Failed,
}

#[derive(Clone, Copy)]
enum OAuthResponsePolicy {
    Management,
    CallbackAcknowledgement,
}

impl ManagedCpaRuntime {
    pub fn is_managed_oauth(&self) -> bool {
        self.spec.managed_oauth.is_some()
    }

    /// Create a separate process/store for a pending login. The returned instance can be
    /// retained after Save; its tokens never need to be copied to a second refresh owner.
    pub fn fork_managed_oauth(
        &self,
        instance_id: String,
        auth_dir: PathBuf,
        kind: CpaAccountKind,
    ) -> Result<Self, CpaLifecycleError> {
        let mut spec = self.spec.clone();
        spec.state_root = auth_dir
            .parent()
            .ok_or(CpaLifecycleError::InvalidSpec)?
            .join("runtime");
        spec.instance_id = instance_id;
        spec.auth_dir = auth_dir;
        spec.borrowed_codex_auth = None;
        spec.borrowed_claude_auth = None;
        spec.managed_oauth = Some(kind);
        spec.bindings.retain(|binding| binding.account_kind == kind);
        let mut runtime = Self::with_components(
            spec,
            self.catalog.clone(),
            self.locator.clone(),
            self.backend.clone(),
            self.control.clone(),
        )?;
        runtime.diagnostics = self.diagnostics.clone();
        runtime.proxy_environment = self.proxy_environment.clone();
        Ok(runtime)
    }

    pub(super) fn managed_oauth_source(&self) -> Option<ManagedOAuthCredentialSource> {
        self.spec
            .managed_oauth
            .map(|kind| ManagedOAuthCredentialSource {
                kind,
                auth_dir: self.spec.auth_dir.clone(),
            })
    }

    /// Safe account identity only: no file names, addresses, tokens, or raw management JSON.
    pub fn managed_credentials(
        &self,
    ) -> Result<Vec<CpaManagedCredentialSummary>, CpaLifecycleError> {
        let source = self
            .managed_oauth_source()
            .ok_or(CpaLifecycleError::InvalidSpec)?;
        source
            .list()
            .map(|entries| entries.iter().map(|entry| entry.summary()).collect())
    }

    pub(super) fn inspect_managed_authentication(
        &self,
        evidence: &crate::CpaManagedEvidence,
    ) -> Result<(), CpaLifecycleError> {
        // A complete immutable target observation avoids waiting for a slow
        // lifecycle owner. This is still a real authenticated refresh-status read.
        let observed = self.observation.lock().live.clone();
        let Some(observed) = observed else {
            return Ok(());
        };
        let (generation, ticket) = {
            let mut admission = self.admission.state.lock();
            crate::request_context::check()?;
            admission.managed_status_sequence = admission.managed_status_sequence.wrapping_add(1);
            (admission.auth_generation, admission.managed_status_sequence)
        };
        let result = (|| {
            let path = format!(
                "/v8/management/credentials?name={}",
                percent_encode_query(evidence.stock_file_name())
            );
            let response = request(LoopbackRequest {
                address: observed.address,
                method: "GET",
                path: &path,
                authorization: Some(&observed.management),
                body: &[],
                timeout: self.spec.control_timeout,
            })
            .map_err(|_| CpaLifecycleError::ControlUnavailable)?;
            if response.status != 200 || !self.epochs.matches(observed.epochs.0, observed.epochs.1)
            {
                return Err(CpaLifecycleError::ControlUnavailable);
            }
            validate_response_version(
                OAuthResponsePolicy::Management,
                response.header("x-cpa-version"),
                &observed.version,
            )?;
            let value = serde_json::from_slice(&response.body)
                .map_err(|_| CpaLifecycleError::UnsafeControlResponse)?;
            validate_credential_status(&value, evidence.stock_file_name(), evidence.kind())
        })();
        let mut admission = self.admission.state.lock();
        crate::request_context::check()?;
        if !self.epochs.matches(observed.epochs.0, observed.epochs.1) {
            return Err(CpaLifecycleError::ControlUnavailable);
        }
        if result.is_ok() {
            if admission
                .managed_status_success
                .as_ref()
                .is_none_or(|previous| previous.ticket < ticket)
            {
                admission.managed_status_success = Some(super::admission::ManagedStatusSuccess {
                    ticket,
                    binding: evidence.evidence_digest().clone(),
                    epochs: observed.epochs,
                });
            }
        } else if admission
            .managed_status_success
            .as_ref()
            .is_some_and(|confirmed| {
                confirmed.ticket > ticket
                    && &confirmed.binding == evidence.evidence_digest()
                    && confirmed.epochs == observed.epochs
            })
        {
            // Use the newer completed proof for this exact login and runtime.
            // Returning the obsolete failure would let a maintenance caller
            // suspend the source again outside this guarded publication boundary.
            return Ok(());
        } else if admission.auth_generation == generation {
            self.health_invalidated
                .store(true, std::sync::atomic::Ordering::Release);
            if matches!(
                result,
                Err(CpaLifecycleError::ManagedOAuthAuthenticationRequired)
            ) {
                self.reject_preparation_locked(&mut admission, generation);
            }
        }
        result
    }

    pub fn oauth_start(&self) -> Result<CpaOAuthLogin, CpaLifecycleError> {
        let kind = self
            .spec
            .managed_oauth
            .ok_or(CpaLifecycleError::InvalidSpec)?;
        self.start()?;
        let mut inner = self.lock_lifecycle()?;
        if inner.oauth_state.is_some()
            || !self
                .managed_oauth_source()
                .ok_or(CpaLifecycleError::InvalidSpec)?
                .list()?
                .is_empty()
        {
            return Err(CpaLifecycleError::InvalidOAuthSession);
        }
        // Stock CPA's web UI forwarder binds all interfaces. The protected HiRoute callback
        // input is used instead; this request must never opt into that forwarder.
        let path = format!(
            "/v8/management/oauth/auth-url?provider={}&is_webui=false",
            kind.stock_provider()
        );
        let value = self.oauth_request_locked(
            &mut inner,
            "GET",
            &path,
            &[],
            OAuthResponsePolicy::Management,
        )?;
        let login = parse_login(kind, &value)?;
        inner.oauth_state = Some(login.state.clone());
        inner.oauth_callback_submitted = false;
        Ok(login)
    }

    pub fn oauth_status(&self, state: &str) -> Result<CpaOAuthStatus, CpaLifecycleError> {
        let mut inner = self.lock_lifecycle()?;
        validate_session(&inner, state)?;
        let path = format!(
            "/v8/management/oauth/status?state={}",
            percent_encode_query(state)
        );
        let value = self.oauth_request_locked(
            &mut inner,
            "GET",
            &path,
            &[],
            OAuthResponsePolicy::Management,
        )?;
        match value.get("status").and_then(Value::as_str) {
            Some("wait") => Ok(CpaOAuthStatus::Pending),
            Some("ok") => {
                // OAuth completion is not routing admission; it only establishes a readable
                // credential. The existing Check -> Save projection grants execution later.
                self.managed_oauth_source()
                    .ok_or(CpaLifecycleError::InvalidSpec)?
                    .inspect()?;
                Ok(CpaOAuthStatus::Complete)
            }
            Some("error") => Ok(CpaOAuthStatus::Failed),
            _ => Err(CpaLifecycleError::UnsafeControlResponse),
        }
    }

    /// `callback` is protected input containing the redirect URL or authorization code.
    /// It must not be placed in ordinary requests, diagnostics, or command arguments.
    pub fn oauth_submit_callback(
        &self,
        state: &str,
        callback: &str,
    ) -> Result<(), CpaLifecycleError> {
        let kind = self
            .spec
            .managed_oauth
            .ok_or(CpaLifecycleError::InvalidSpec)?;
        let mut inner = self.lock_lifecycle()?;
        validate_session(&inner, state)?;
        if inner.oauth_callback_submitted {
            return Err(CpaLifecycleError::InvalidOAuthSession);
        }
        let code = callback_code(kind, state, callback)?;
        #[derive(serde::Serialize)]
        struct CallbackInput<'a> {
            provider: &'a str,
            state: &'a str,
            code: &'a str,
        }
        let body = Zeroizing::new(
            serde_json::to_vec(&CallbackInput {
                provider: kind.stock_provider(),
                state,
                code: code.as_str(),
            })
            .map_err(|_| CpaLifecycleError::InvalidOAuthSession)?,
        );
        // An ambiguous response may follow a successful one-time code exchange.
        // Close input admission before sending; never replay the submitted code.
        inner.oauth_callback_submitted = true;
        let value = self.oauth_request_locked(
            &mut inner,
            "POST",
            "/v8/management/oauth/callback",
            &body,
            OAuthResponsePolicy::CallbackAcknowledgement,
        )?;
        if value.get("status").and_then(Value::as_str) != Some("ok") {
            return Err(CpaLifecycleError::UnsafeControlResponse);
        }
        Ok(())
    }

    /// Cancels CPA's pending session. The registry must then stop this pending runtime
    /// before deleting its owned directory, so an in-flight token exchange cannot write late.
    pub fn oauth_cancel(&self, state: &str) -> Result<(), CpaLifecycleError> {
        let mut inner = self.lock_lifecycle()?;
        validate_session(&inner, state)?;
        let path = format!(
            "/v8/management/oauth/session?state={}",
            percent_encode_query(state)
        );
        let result = self.oauth_request_locked(
            &mut inner,
            "DELETE",
            &path,
            &[],
            OAuthResponsePolicy::Management,
        );
        // Revoke local input admission even when the control request failed; the owner
        // still has to stop this process before removing the pending credential store.
        inner.oauth_state = None;
        inner.oauth_callback_submitted = true;
        let value = result?;
        if value.get("status").and_then(Value::as_str) == Some("ok") {
            Ok(())
        } else {
            Err(CpaLifecycleError::UnsafeControlResponse)
        }
    }

    fn oauth_request_locked(
        &self,
        inner: &mut RuntimeInner,
        method: &'static str,
        path: &str,
        body: &[u8],
        policy: OAuthResponsePolicy,
    ) -> Result<Value, CpaLifecycleError> {
        if !self.is_managed_oauth() {
            return Err(CpaLifecycleError::InvalidSpec);
        }
        if matches!(policy, OAuthResponsePolicy::CallbackAcknowledgement)
            && (method != "POST" || path != "/v8/management/oauth/callback")
        {
            return Err(CpaLifecycleError::InvalidSpec);
        }
        // OAuth session state belongs to this exact CPA process. A control read,
        // callback or cancel cannot recover that state by launching another writer.
        // Only explicit login/start or enabled saved-source admission may start CPA.
        match self.health_locked(inner)? {
            super::CpaHealth::Ready { .. } => {}
            super::CpaHealth::Stopped { .. } => return Err(CpaLifecycleError::NotStarted),
            _ => return Err(CpaLifecycleError::ControlUnavailable),
        }
        let live = inner.live.as_ref().ok_or(CpaLifecycleError::NotStarted)?;
        let response = request(LoopbackRequest {
            address: live.address,
            method,
            path,
            authorization: Some(&live.secrets.management),
            body,
            timeout: self.spec.control_timeout,
        })
        .map_err(|_| CpaLifecycleError::ControlUnavailable)?;
        if response.status != 200 {
            return Err(CpaLifecycleError::ControlUnavailable);
        }
        validate_response_version(
            policy,
            response.header("x-cpa-version"),
            &live.artifact.version().to_string(),
        )?;
        serde_json::from_slice(&response.body).map_err(|_| CpaLifecycleError::UnsafeControlResponse)
    }
}

fn validate_response_version(
    policy: OAuthResponsePolicy,
    reported: Option<&str>,
    expected: &str,
) -> Result<(), CpaLifecycleError> {
    match reported {
        Some(version) if version.trim().trim_start_matches('v') == expected => Ok(()),
        // Pinned CPA registers callback outside management.Middleware, which
        // injects this header. The same process was authenticated/version-checked
        // by health_locked immediately before this request. All other management
        // responses still require the header, and a present mismatch is rejected.
        None if matches!(policy, OAuthResponsePolicy::CallbackAcknowledgement) => Ok(()),
        _ => Err(CpaLifecycleError::UnsafeControlResponse),
    }
}

fn validate_session(inner: &RuntimeInner, state: &str) -> Result<(), CpaLifecycleError> {
    if !valid_state(state) || inner.oauth_state.as_deref() != Some(state) {
        return Err(CpaLifecycleError::InvalidOAuthSession);
    }
    Ok(())
}

fn validate_credential_status(
    value: &Value,
    name: &str,
    kind: CpaAccountKind,
) -> Result<(), CpaLifecycleError> {
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or(CpaLifecycleError::UnsafeControlResponse)?;
    if files.is_empty() {
        return Err(CpaLifecycleError::ManagedOAuthCredentialsMissing);
    }
    if files.len() != 1 {
        return Err(CpaLifecycleError::UnsafeControlResponse);
    }
    let entry = &files[0];
    if entry.get("name").and_then(Value::as_str) != Some(name)
        || entry.get("provider").and_then(Value::as_str) != Some(kind.stock_provider())
        || entry.get("source").and_then(Value::as_str) != Some("file")
        || entry.get("runtime_only").and_then(Value::as_bool) != Some(false)
        || entry.get("account_type").and_then(Value::as_str) != Some("oauth")
    {
        return Err(CpaLifecycleError::UnsafeControlResponse);
    }
    if entry
        .get("status_message")
        .and_then(Value::as_str)
        .is_some_and(crate::managed_oauth::authentication_required)
    {
        return Err(CpaLifecycleError::ManagedOAuthAuthenticationRequired);
    }
    if entry.get("status").and_then(Value::as_str) != Some("active")
        || entry.get("disabled").and_then(Value::as_bool) != Some(false)
        || entry.get("unavailable").and_then(Value::as_bool) != Some(false)
    {
        return Err(CpaLifecycleError::ControlUnavailable);
    }
    Ok(())
}

fn valid_state(state: &str) -> bool {
    !state.is_empty()
        && state.len() <= 256
        && state
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn parse_login(kind: CpaAccountKind, value: &Value) -> Result<CpaOAuthLogin, CpaLifecycleError> {
    let invalid = || CpaLifecycleError::UnsafeControlResponse;
    let state = value
        .get("state")
        .and_then(Value::as_str)
        .filter(|state| valid_state(state))
        .ok_or_else(invalid)?;
    let raw_url = value
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.len() <= 16 * 1024)
        .ok_or_else(invalid)?;
    let url = Url::parse(raw_url).map_err(|_| invalid())?;
    let host = match kind {
        CpaAccountKind::Codex => "auth.openai.com",
        CpaAccountKind::Claude => "claude.ai",
    };
    if value.get("status").and_then(Value::as_str) != Some("ok")
        || url.scheme() != "https"
        || url.host_str() != Some(host)
        || url.path() != "/oauth/authorize"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || raw_url.contains(['\r', '\n', '\0'])
        || url.query_pairs().any(|(key, _)| {
            !matches!(
                key.as_ref(),
                "client_id"
                    | "response_type"
                    | "redirect_uri"
                    | "scope"
                    | "state"
                    | "code_challenge"
                    | "code_challenge_method"
                    | "prompt"
                    | "id_token_add_organizations"
                    | "codex_cli_simplified_flow"
                    | "code"
            )
        })
        || url
            .query_pairs()
            .filter(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>()
            != [state]
        || url
            .query_pairs()
            .filter(|(key, _)| key == "code_challenge_method")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>()
            != ["S256"]
    {
        return Err(invalid());
    }
    Ok(CpaOAuthLogin {
        authorization_url: raw_url.to_owned(),
        state: state.to_owned(),
    })
}

fn callback_code(
    kind: CpaAccountKind,
    state: &str,
    input: &str,
) -> Result<Zeroizing<String>, CpaLifecycleError> {
    let invalid = || CpaLifecycleError::InvalidOAuthSession;
    let input = input.trim();
    if input.is_empty() || input.len() > 16 * 1024 || input.contains(['\r', '\n', '\0']) {
        return Err(invalid());
    }
    let code = if input.contains("://") {
        let url = Url::parse(input).map_err(|_| invalid())?;
        let (port, path) = match kind {
            CpaAccountKind::Codex => (1455, "/auth/callback"),
            CpaAccountKind::Claude => (54545, "/callback"),
        };
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
            || url.port() != Some(port)
            || url.path() != path
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        let states = url
            .query_pairs()
            .filter(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        let codes = url
            .query_pairs()
            .filter(|(key, _)| key == "code")
            .map(|(_, value)| value.into_owned())
            .collect::<Vec<_>>();
        if states != [state] || codes.len() != 1 {
            return Err(invalid());
        }
        codes.into_iter().next().ok_or_else(invalid)?
    } else if let Some((code, appended_state)) = input.split_once('#') {
        if kind != CpaAccountKind::Claude || appended_state != state {
            return Err(invalid());
        }
        code.to_owned()
    } else {
        input.to_owned()
    };
    if code.is_empty() || code.contains(['\r', '\n', '\0']) {
        return Err(invalid());
    }
    Ok(Zeroizing::new(code))
}

#[cfg(test)]
mod tests;
