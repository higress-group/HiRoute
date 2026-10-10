//! Private login lifecycle. The manifest identifies resources, never grants routing authority.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::config::{SecretText, ensure_private_dir, private_atomic_write, validate_private_file};
use crate::{CpaAccountKind, CpaLifecycleError, ManagedCpaRuntime};

const SCHEMA: &str = "hiroute.cpa-managed-login/v1";
const LOGIN_WINDOW_SECONDS: u64 = 5 * 60;
const MAX_SESSIONS: usize = 32;

#[cfg(test)]
#[path = "managed_sessions/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaLoginState {
    Pending,
    Authorized,
    Cancelled,
    Failed,
    Expired,
    Forgotten,
}

#[derive(Clone, Debug)]
pub struct CpaLoginSession {
    pub login_ref: String,
    pub kind: CpaAccountKind,
    pub state: CpaLoginState,
    pub account_ref: Option<String>,
}

impl CpaLoginSession {
    pub fn candidate_ref(&self) -> String {
        format!(
            "candidate/cpa/{}/managed/{}",
            self.kind.stock_provider(),
            self.login_ref
        )
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LoginRecord {
    schema: String,
    login_ref: String,
    kind: CpaAccountKind,
    state: CpaLoginState,
    created_at: u64,
    expires_at: u64,
    oauth_state: Option<String>,
    account_ref: Option<String>,
}

impl LoginRecord {
    fn safe(&self) -> CpaLoginSession {
        CpaLoginSession {
            login_ref: self.login_ref.clone(),
            kind: self.kind,
            state: self.state,
            account_ref: self.account_ref.clone(),
        }
    }
}

struct Session {
    record: LoginRecord,
    runtime: Arc<ManagedCpaRuntime>,
}

pub(crate) struct ManagedLoginRegistry {
    root: PathBuf,
    sessions: Mutex<BTreeMap<String, Session>>,
}

impl ManagedLoginRegistry {
    pub(crate) fn load(
        root: PathBuf,
        templates: &[Arc<ManagedCpaRuntime>],
    ) -> Result<Self, CpaLifecycleError> {
        let registry = Self {
            root,
            sessions: Mutex::new(BTreeMap::new()),
        };
        if !registry.root.exists() {
            return Ok(registry);
        }
        private_session_dir(&registry.root)?;
        let mut sessions = registry.sessions.lock();
        for entry in std::fs::read_dir(&registry.root).map_err(|_| CpaLifecycleError::OwnerState)? {
            let entry = entry.map_err(|_| CpaLifecycleError::OwnerState)?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_login_ref(&id) || sessions.len() >= MAX_SESSIONS {
                continue;
            }
            // Preserve damaged or unavailable resources in place. A broken optional login
            // cannot hide an independently valid sibling, and absent sessions cannot route.
            if let Ok(Some(session)) = registry.restore_session(templates, &id) {
                sessions.insert(id, session);
            }
        }
        drop(sessions);
        Ok(registry)
    }

    fn restore_session(
        &self,
        templates: &[Arc<ManagedCpaRuntime>],
        id: &str,
    ) -> Result<Option<Session>, CpaLifecycleError> {
        private_session_dir(&self.session_dir(id))?;
        let path = self.session_dir(id).join("login.json");
        if !path.exists() {
            return Ok(None);
        }
        validate_private_file(&path)?;
        if std::fs::metadata(&path)
            .map_err(|_| CpaLifecycleError::OwnerState)?
            .len()
            > 16384
        {
            return Err(CpaLifecycleError::OwnerState);
        }
        let record: LoginRecord = serde_json::from_slice(
            &std::fs::read(path).map_err(|_| CpaLifecycleError::OwnerState)?,
        )
        .map_err(|_| CpaLifecycleError::OwnerState)?;
        if record.schema != SCHEMA
            || record.login_ref != id
            || record.expires_at < record.created_at
            || record
                .oauth_state
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 4096)
            || (record.state == CpaLoginState::Authorized
                && record.account_ref.as_ref().is_none_or(|a| {
                    !a.strip_prefix("account/cpa/")
                        .is_some_and(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
                }))
        {
            return Err(CpaLifecycleError::OwnerState);
        }
        let runtime = self.make_runtime(templates, &record)?;
        let mut session = Session { record, runtime };
        if session.record.state == CpaLoginState::Pending {
            // Stop only an authenticated existing owner. Recovery cannot start a new
            // refresh writer before saved management has granted execution.
            session.runtime.stop_existing_managed_process()?;
            clear_auth(&self.session_dir(id).join("auth"))?;
            session.record.state = CpaLoginState::Expired;
            session.record.oauth_state = None;
            self.save(&session.record)?;
        } else if matches!(
            session.record.state,
            CpaLoginState::Authorized | CpaLoginState::Failed
        ) {
            // Saved management chooses which refresh process may subsequently run.
            session.runtime.stop_existing_managed_process()?;
        }
        Ok(Some(session))
    }

    pub(crate) fn start(
        &self,
        templates: &[Arc<ManagedCpaRuntime>],
        kind: CpaAccountKind,
    ) -> Result<(CpaLoginSession, String), CpaLifecycleError> {
        let mut sessions = self.sessions.lock();
        for session in sessions
            .values_mut()
            .filter(|s| s.record.kind == kind && s.record.state == CpaLoginState::Pending)
        {
            if now()? >= session.record.expires_at {
                self.stop_session(session, CpaLoginState::Expired)?;
            }
        }
        if sessions.len() >= MAX_SESSIONS {
            let removable = sessions
                .values()
                .filter(|s| {
                    matches!(
                        s.record.state,
                        CpaLoginState::Cancelled
                            | CpaLoginState::Expired
                            | CpaLoginState::Forgotten
                    )
                })
                .min_by_key(|s| s.record.created_at)
                .map(|s| s.record.login_ref.clone());
            if let Some(id) = removable {
                let path = self.session_dir(&id);
                private_session_dir(&path)?;
                std::fs::remove_dir_all(path).map_err(|_| CpaLifecycleError::OwnerState)?;
                sessions.remove(&id);
            }
        }
        if sessions.len() >= MAX_SESSIONS
            || sessions
                .values()
                .any(|s| s.record.kind == kind && s.record.state == CpaLoginState::Pending)
        {
            return Err(CpaLifecycleError::AlreadyOwned);
        }
        private_session_dir(&self.root)?;
        let id = format!("login-{}", SecretText::generate()?.expose());
        let now = now()?;
        let mut record = LoginRecord {
            schema: SCHEMA.into(),
            login_ref: id.clone(),
            kind,
            state: CpaLoginState::Pending,
            created_at: now,
            expires_at: now + LOGIN_WINDOW_SECONDS,
            oauth_state: None,
            account_ref: None,
        };
        private_session_dir(&self.session_dir(&id))?;
        self.save(&record)?;
        let runtime = self.make_runtime(templates, &record)?;
        // A logged-in but unchecked credential is never an execution grant.
        runtime.suspend_subscription_execution();
        let login = match runtime.oauth_start() {
            Ok(login) => login,
            Err(error) => {
                let _ = runtime.shutdown();
                record.state = CpaLoginState::Failed;
                self.save(&record)?;
                sessions.insert(id, Session { record, runtime });
                return Err(error);
            }
        };
        record.oauth_state = Some(login.state);
        self.save(&record)?;
        let safe = record.safe();
        sessions.insert(id, Session { record, runtime });
        Ok((safe, login.authorization_url))
    }

    pub(crate) fn list(&self, kind: CpaAccountKind) -> Vec<CpaLoginSession> {
        self.sessions
            .lock()
            .values()
            .filter(|s| s.record.kind == kind)
            .map(|s| s.record.safe())
            .collect()
    }

    pub(crate) fn status(&self, id: &str) -> Result<CpaLoginSession, CpaLifecycleError> {
        let mut sessions = self.sessions.lock();
        let session = sessions
            .get_mut(id)
            .ok_or(CpaLifecycleError::InvalidSourceManagement)?;
        if session.record.state != CpaLoginState::Pending {
            return Ok(session.record.safe());
        }
        if now()? >= session.record.expires_at {
            self.stop_session(session, CpaLoginState::Expired)?;
            return Ok(session.record.safe());
        }
        let oauth_state = session
            .record
            .oauth_state
            .as_deref()
            .ok_or(CpaLifecycleError::OwnerState)?;
        match session.runtime.oauth_status(oauth_state)? {
            crate::CpaOAuthStatus::Pending => {}
            crate::CpaOAuthStatus::Complete => {
                let credentials = session.runtime.managed_credentials()?;
                if credentials.len() != 1 || credentials[0].kind != session.record.kind {
                    self.stop_session(session, CpaLoginState::Failed)?;
                } else {
                    // Unsaved authorizations do not need a background refresh process.
                    // Keep Pending on a stop failure so maintenance retries cleanup.
                    session.runtime.stop_existing_managed_process()?;
                    let mut authorized = session.record.clone();
                    authorized.account_ref = Some(credentials[0].account_ref.clone());
                    authorized.state = CpaLoginState::Authorized;
                    authorized.oauth_state = None;
                    self.save(&authorized)?;
                    session.record = authorized;
                }
            }
            crate::CpaOAuthStatus::Failed => self.stop_session(session, CpaLoginState::Failed)?,
        }
        Ok(session.record.safe())
    }

    /// Daemon-owned housekeeping continues even when the login client has disconnected.
    /// A failed optional session must not prevent another provider from completing cleanup.
    pub(crate) fn maintain(&self) -> Result<(), CpaLifecycleError> {
        let pending = self
            .sessions
            .lock()
            .values()
            .filter(|session| session.record.state == CpaLoginState::Pending)
            .map(|session| session.record.login_ref.clone())
            .collect::<Vec<_>>();
        let mut first_error = None;
        for id in pending {
            if let Err(error) = self.status(&id) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn callback(&self, id: &str, value: &str) -> Result<(), CpaLifecycleError> {
        let sessions = self.sessions.lock();
        let session = sessions
            .get(id)
            .ok_or(CpaLifecycleError::InvalidSourceManagement)?;
        if session.record.state != CpaLoginState::Pending || now()? >= session.record.expires_at {
            return Err(CpaLifecycleError::StaleSourceManagement);
        }
        let state = session
            .record
            .oauth_state
            .as_deref()
            .ok_or(CpaLifecycleError::OwnerState)?;
        session.runtime.oauth_submit_callback(state, value)
    }

    pub(crate) fn cancel(
        &self,
        id: &str,
        forget: bool,
    ) -> Result<CpaLoginSession, CpaLifecycleError> {
        let mut sessions = self.sessions.lock();
        let session = sessions
            .get_mut(id)
            .ok_or(CpaLifecycleError::InvalidSourceManagement)?;
        if !forget && session.record.state == CpaLoginState::Authorized {
            return Err(CpaLifecycleError::InvalidSourceManagement);
        }
        self.stop_session(
            session,
            if forget {
                CpaLoginState::Forgotten
            } else {
                CpaLoginState::Cancelled
            },
        )?;
        Ok(session.record.safe())
    }

    pub(crate) fn for_candidate(&self, candidate: &str) -> Option<Arc<ManagedCpaRuntime>> {
        let sessions = crate::request_context::lock(&self.sessions).ok()?;
        sessions
            .values()
            .find(|session| {
                session.record.state == CpaLoginState::Authorized
                    && session.record.safe().candidate_ref() == candidate
            })
            .map(|session| session.runtime.clone())
    }

    pub(crate) fn session(&self, id: &str) -> Option<CpaLoginSession> {
        self.sessions.lock().get(id).map(|s| s.record.safe())
    }

    pub(crate) fn all_runtimes(&self) -> Vec<Arc<ManagedCpaRuntime>> {
        self.sessions
            .lock()
            .values()
            .map(|s| s.runtime.clone())
            .collect()
    }

    fn stop_session(
        &self,
        session: &mut Session,
        terminal: CpaLoginState,
    ) -> Result<(), CpaLifecycleError> {
        session.runtime.suspend_subscription_execution();
        // Reconstructed runtimes may have a live orphan, but stopping or forgetting an
        // inactive login must never spawn a process or rotate its refresh token.
        // Stopping the process also cancels CPA's in-memory OAuth session and tasks.
        session.runtime.stop_existing_managed_process()?;
        // Commit the terminal state only after deletion. An interrupted cleanup remains
        // retryable through the original session and cannot leave a false success receipt.
        clear_auth(&self.session_dir(&session.record.login_ref).join("auth"))?;
        session.record.state = terminal;
        session.record.oauth_state = None;
        session.record.account_ref = None;
        self.save(&session.record)
    }

    fn make_runtime(
        &self,
        templates: &[Arc<ManagedCpaRuntime>],
        record: &LoginRecord,
    ) -> Result<Arc<ManagedCpaRuntime>, CpaLifecycleError> {
        let template = templates
            .iter()
            .find(|runtime| runtime.managed_kind() == Some(record.kind))
            .ok_or(CpaLifecycleError::InvalidSpec)?;
        let runtime = template.fork_managed_oauth(
            record.login_ref.clone(),
            self.session_dir(&record.login_ref).join("auth"),
            record.kind,
        )?;
        runtime.suspend_subscription_execution();
        Ok(Arc::new(runtime))
    }

    fn session_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn save(&self, record: &LoginRecord) -> Result<(), CpaLifecycleError> {
        let bytes = serde_json::to_vec(record).map_err(|_| CpaLifecycleError::OwnerState)?;
        private_atomic_write(
            &self.session_dir(&record.login_ref).join("login.json"),
            &bytes,
        )?;
        Ok(())
    }
}

fn valid_login_ref(value: &str) -> bool {
    value.strip_prefix("login-").is_some_and(|id| {
        id.len() == 43
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    })
}

fn now() -> Result<u64, CpaLifecycleError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| CpaLifecycleError::OwnerState)
}

fn clear_auth(path: &Path) -> Result<(), CpaLifecycleError> {
    if !path.exists() {
        return Ok(());
    }
    private_session_dir(path)?;
    // Stop was confirmed before this exact owner-only directory is removed. Never follow links.
    std::fs::remove_dir_all(path).map_err(|_| CpaLifecycleError::OwnerState)
}

fn private_session_dir(path: &Path) -> Result<(), CpaLifecycleError> {
    if let Ok(metadata) = std::fs::symlink_metadata(path)
        && (!metadata.is_dir() || metadata.file_type().is_symlink())
    {
        return Err(CpaLifecycleError::OwnerState);
    }
    ensure_private_dir(path)?;
    Ok(())
}
