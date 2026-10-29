//! Hash-only credentials and bounded in-memory sessions. Never log request bodies.
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    net::IpAddr,
    path::{Path, PathBuf},
    time::Instant,
};
use zeroize::Zeroizing;
mod launch;
mod store;
pub use launch::LaunchTarget;
pub(crate) use launch::authority;
#[cfg(test)]
mod tests;

const LAUNCH_TTL: u64 = 120;
const IDLE_TTL: u64 = 12 * 60 * 60;
const ABSOLUTE_TTL: u64 = 7 * 24 * 60 * 60;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    Required,
    RateLimited,
    Invalid,
    Capacity,
    Unavailable,
    ListenerUnavailable,
}
impl AuthError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Required => "CONSOLE_AUTH_REQUIRED",
            Self::RateLimited => "CONSOLE_RATE_LIMITED",
            Self::Invalid => "CONSOLE_CREDENTIAL_INVALID",
            Self::Capacity => "CONSOLE_SESSION_LIMIT",
            Self::Unavailable => "CONSOLE_AUTH_UNAVAILABLE",
            Self::ListenerUnavailable => "CONSOLE_LISTENER_UNAVAILABLE",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub id: String,
    pub name: String,
    pub created: u64,
    pub expires: Option<u64>,
    pub revoked: bool,
}
#[derive(Serialize)]
pub struct CreatedToken {
    #[serde(flatten)]
    pub metadata: TokenMetadata,
    pub token: String,
}
#[derive(Clone, Serialize)]
pub struct SessionIdentity {
    pub id: String,
    pub credential_kind: &'static str,
    pub token_id: Option<String>,
    pub expires: u64,
}
#[derive(Serialize)]
pub struct SessionExchange {
    pub bearer: String,
    pub session: SessionIdentity,
}
struct Session {
    identity: SessionIdentity,
    created: u64,
    last_used: u64,
    last_access: Instant,
}
#[derive(Serialize)]
pub(super) struct AuditEvent<'a> {
    at: u64,
    event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credential_kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation_id: Option<&'a str>,
}
pub struct ConsoleAuth {
    root: PathBuf,
    tokens: Vec<store::StoredToken>,
    launches: BTreeMap<String, launch::Launch>,
    sessions: BTreeMap<String, Session>,
    failures: VecDeque<(u64, IpAddr)>,
}
impl ConsoleAuth {
    pub fn open(data_root: &Path) -> Result<Self, AuthError> {
        let root = data_root.join("console");
        store::prepare(&root)?;
        let tokens = store::load(&root)?;
        Ok(Self {
            root,
            tokens,
            launches: BTreeMap::new(),
            sessions: BTreeMap::new(),
            failures: VecDeque::new(),
        })
    }
    pub fn create_launch(&mut self, target: LaunchTarget) -> Result<Zeroizing<String>, AuthError> {
        let now = now()?;
        self.launches.retain(|_, entry| entry.expires > now);
        if self.launches.len() >= 32 {
            return Err(AuthError::Capacity);
        }
        let secret = random()?;
        self.credential_event("launch_created", "launch", None)?;
        self.launches.insert(
            hash(&secret),
            launch::Launch {
                expires: now + LAUNCH_TTL,
                target,
            },
        );
        Ok(secret)
    }
    pub fn create_token(
        &mut self,
        name: String,
        days: Option<u32>,
    ) -> Result<CreatedToken, AuthError> {
        if !valid_name(&name) || days.is_some_and(|d| d == 0 || d > 3650) {
            return Err(AuthError::Invalid);
        }
        if self.tokens.len() >= store::MAX_TOKENS {
            return Err(AuthError::Capacity);
        }
        let created = now()?;
        let metadata = TokenMetadata {
            id: random()?.to_string(),
            name,
            created,
            expires: Some(created + days.unwrap_or(90) as u64 * 86400),
            revoked: false,
        };
        let secret = random()?;
        let mut tokens = self.tokens.clone();
        tokens.push(store::StoredToken {
            metadata: metadata.clone(),
            sha256: hash(&secret),
        });
        store::save(&self.root, &tokens)?;
        self.tokens = tokens;
        self.credential_event("token_created", "token", Some(&metadata.id))?;
        Ok(CreatedToken {
            metadata,
            token: secret.to_string(),
        })
    }
    pub fn list_tokens(&self) -> Vec<TokenMetadata> {
        self.tokens.iter().map(|t| t.metadata.clone()).collect()
    }
    pub fn revoke_token(&mut self, id: &str) -> Result<(), AuthError> {
        let mut tokens = self.tokens.clone();
        tokens
            .iter_mut()
            .find(|t| t.metadata.id == id)
            .ok_or(AuthError::Invalid)?
            .metadata
            .revoked = true;
        store::save(&self.root, &tokens)?;
        self.tokens = tokens;
        self.credential_event("token_revoked", "token", Some(id))?;
        let keys: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.identity.token_id.as_deref() == Some(id))
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            self.remove_session(&key, "session_revoked")?;
        }
        Ok(())
    }
    pub fn exchange_launch(
        &mut self,
        code: &str,
        peer: IpAddr,
        origin: &str,
    ) -> Result<SessionExchange, AuthError> {
        self.rate(peer)?;
        let now = now()?;
        if !valid_secret(code) {
            return self.failed(peer);
        }
        let digest = hash(code);
        if !self.launches.get(&digest).is_some_and(|entry| {
            entry.expires > now
                && entry.target.origin == origin
                && (!entry.target.loopback_only || peer.is_loopback())
        }) {
            return self.failed(peer);
        }
        // Consume before session creation: a failed/cancelled exchange cannot replay.
        self.launches.remove(&digest);
        self.new_session("launch", None, now)
    }
    pub fn exchange_token(
        &mut self,
        token: &str,
        peer: IpAddr,
    ) -> Result<SessionExchange, AuthError> {
        self.rate(peer)?;
        let now = now()?;
        if !valid_secret(token) {
            return self.failed(peer);
        }
        let digest = hash(token);
        let Some(id) = self
            .tokens
            .iter()
            .find(|t| {
                t.sha256 == digest
                    && !t.metadata.revoked
                    && t.metadata.expires.is_none_or(|e| e > now)
            })
            .map(|t| t.metadata.id.clone())
        else {
            return self.failed(peer);
        };
        self.new_session("token", Some(id), now)
    }
    fn new_session(
        &mut self,
        kind: &'static str,
        token_id: Option<String>,
        now: u64,
    ) -> Result<SessionExchange, AuthError> {
        self.expire(now)?;
        let secret = random()?;
        let identity = SessionIdentity {
            id: random()?.to_string(),
            credential_kind: kind,
            token_id,
            expires: now + IDLE_TTL,
        };
        if self.sessions.len() >= 16 {
            let oldest = self
                .sessions
                .iter()
                .min_by_key(|(_, session)| session.last_access)
                .map(|(key, _)| key.clone())
                .ok_or(AuthError::Unavailable)?;
            self.remove_session(&oldest, "session_evicted")?;
        }
        self.session_event("session_created", &identity, now)?;
        self.sessions.insert(
            hash(&secret),
            Session {
                identity: identity.clone(),
                created: now,
                last_used: now,
                last_access: Instant::now(),
            },
        );
        Ok(SessionExchange {
            bearer: secret.to_string(),
            session: identity,
        })
    }
    pub fn authenticate(
        &mut self,
        headers: &HeaderMap,
        _peer: IpAddr,
    ) -> Result<SessionIdentity, AuthError> {
        let mut values = headers.get_all(http::header::AUTHORIZATION).iter();
        let value = values
            .next()
            .ok_or(AuthError::Required)?
            .to_str()
            .map_err(|_| AuthError::Required)?;
        if values.next().is_some() {
            return Err(AuthError::Required);
        }
        let bearer = value
            .strip_prefix("Bearer ")
            .filter(|v| valid_secret(v))
            .ok_or(AuthError::Required)?;
        self.authenticate_bearer(bearer)
    }
    fn authenticate_bearer(&mut self, bearer: &str) -> Result<SessionIdentity, AuthError> {
        let now = now()?;
        self.expire(now)?;
        let session = self
            .sessions
            .get_mut(&hash(bearer))
            .ok_or(AuthError::Required)?;
        session.last_used = now;
        session.last_access = Instant::now();
        session.identity.expires = (now + IDLE_TTL).min(session.created + ABSOLUTE_TTL);
        Ok(session.identity.clone())
    }
    pub fn logout(&mut self, headers: &HeaderMap, peer: IpAddr) -> Result<(), AuthError> {
        let identity = self.authenticate(headers, peer)?;
        let key = self
            .sessions
            .iter()
            .find(|(_, s)| s.identity.id == identity.id)
            .map(|(k, _)| k.clone())
            .ok_or(AuthError::Required)?;
        self.remove_session(&key, "session_revoked")
    }
    pub fn session_active(&mut self, id: &str) -> bool {
        now().and_then(|n| self.expire(n)).is_ok()
            && self.sessions.values().any(|s| s.identity.id == id)
    }
    fn expire(&mut self, now: u64) -> Result<(), AuthError> {
        let keys: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, s)| {
                now >= s.last_used + IDLE_TTL
                    || now >= s.created + ABSOLUTE_TTL
                    || s.identity.token_id.as_ref().is_some_and(|id| {
                        self.tokens.iter().all(|t| {
                            t.metadata.id != *id
                                || t.metadata.revoked
                                || t.metadata.expires.is_some_and(|e| e <= now)
                        })
                    })
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            self.remove_session(&key, "session_expired")?;
        }
        Ok(())
    }
    fn remove_session(&mut self, key: &str, event: &str) -> Result<(), AuthError> {
        if let Some(session) = self.sessions.remove(key) {
            self.session_event(event, &session.identity, now()?)?;
        }
        Ok(())
    }
    fn credential_event(
        &self,
        event: &str,
        kind: &str,
        token_id: Option<&str>,
    ) -> Result<(), AuthError> {
        store::audit(
            &self.root,
            &AuditEvent {
                at: now()?,
                event,
                session_id: None,
                credential_kind: Some(kind),
                token_id,
                context: None,
                command: None,
                operation_id: None,
            },
        )
    }
    fn session_event(&self, event: &str, s: &SessionIdentity, at: u64) -> Result<(), AuthError> {
        store::audit(
            &self.root,
            &AuditEvent {
                at,
                event,
                session_id: Some(&s.id),
                credential_kind: Some(s.credential_kind),
                token_id: s.token_id.as_deref(),
                context: None,
                command: None,
                operation_id: None,
            },
        )
    }
    /// Fields are supplied from the admitted catalog and validated context, never a body.
    pub fn audit_command(
        &self,
        session: &SessionIdentity,
        context: &str,
        command: &str,
        operation_id: Option<&str>,
    ) -> Result<(), AuthError> {
        if !valid_context(context)
            || command.len() > 80
            || !command
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
            || operation_id.is_some_and(|v| v.len() > 256 || v.chars().any(char::is_control))
        {
            return Err(AuthError::Invalid);
        }
        store::audit(
            &self.root,
            &AuditEvent {
                at: now()?,
                event: "command",
                session_id: Some(&session.id),
                credential_kind: None,
                token_id: None,
                context: Some(context),
                command: Some(command),
                operation_id,
            },
        )
    }
    fn rate(&mut self, peer: IpAddr) -> Result<(), AuthError> {
        let cutoff = now()?.saturating_sub(60);
        while self.failures.front().is_some_and(|(at, _)| *at <= cutoff) {
            self.failures.pop_front();
        }
        // Remote failures must not consume the owner's local recovery budget.
        let realm_failures = self
            .failures
            .iter()
            .filter(|(_, address)| address.is_loopback() == peer.is_loopback())
            .count();
        if realm_failures >= 64 || self.failures.iter().filter(|(_, p)| *p == peer).count() >= 8 {
            Err(AuthError::RateLimited)
        } else {
            Ok(())
        }
    }
    fn failed<T>(&mut self, peer: IpAddr) -> Result<T, AuthError> {
        self.failures.push_back((now()?, peer));
        Err(AuthError::Invalid)
    }
}
pub fn valid_context(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn valid_name(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 80 && !value.chars().any(char::is_control)
}
fn valid_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn random() -> Result<Zeroizing<String>, AuthError> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| AuthError::Unavailable)?;
    Ok(Zeroizing::new(
        bytes.iter().map(|b| format!("{b:02x}")).collect(),
    ))
}
fn now() -> Result<u64, AuthError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| AuthError::Unavailable)
}
