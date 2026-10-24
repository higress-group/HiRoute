//! Irreversible correlation tokens.
//!
//! Business identifiers never appear in a diagnostic record. When a record needs to
//! reference an existing request/attempt/operation, the caller passes the original
//! identifier to [`Tokenizer`], which returns `HMAC-SHA256(key, domain || id)` as a full
//! hex token. The key never leaves the local diagnostics root and is never exported.
//!
//! When the key is missing, unreadable or unsafe, tokenization degrades to `None` with a
//! `correlation_unavailable` reason: the subsystem must not fall back to the raw id, an
//! unsalted hash or a per-process key.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::identity::{CorrelationToken, RandomIdError};

/// Length of the correlation key in bytes.
pub const KEY_LEN: usize = 32;
const TOKEN_DOMAIN_PREFIX: &[u8] = b"hiroute.diagnostic.token.v1";

/// Type domains keep the same original id from producing the same token in two different
/// identifier spaces. Only real relationships are linked with `correlation_link`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrelationDomain {
    ControlRequest,
    ProtectedRegistration,
    ModelRequest,
    Decision,
    Attempt,
    Operation,
    Task,
    Run,
    Plan,
    Binding,
    ContentStream,
    ExternalRequest,
    NativeRoutingHeader,
}

impl CorrelationDomain {
    fn tag(self) -> &'static [u8] {
        match self {
            CorrelationDomain::ControlRequest => b"control_request",
            CorrelationDomain::ProtectedRegistration => b"protected_registration",
            CorrelationDomain::ModelRequest => b"model_request",
            CorrelationDomain::Decision => b"decision",
            CorrelationDomain::Attempt => b"attempt",
            CorrelationDomain::Operation => b"operation",
            CorrelationDomain::Task => b"task",
            CorrelationDomain::Run => b"run",
            CorrelationDomain::Plan => b"plan",
            CorrelationDomain::Binding => b"binding",
            CorrelationDomain::ContentStream => b"content_stream",
            CorrelationDomain::ExternalRequest => b"external_request",
            CorrelationDomain::NativeRoutingHeader => b"native_routing_header",
        }
    }
}

#[derive(Clone)]
pub struct CorrelationKey([u8; KEY_LEN]);

impl std::fmt::Debug for CorrelationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key must never be printable.
        f.write_str("CorrelationKey([redacted])")
    }
}

impl CorrelationKey {
    pub fn generate() -> Result<Self, RandomIdError> {
        let mut bytes = [0u8; KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|_| RandomIdError)?;
        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Raw key material for persistence into `correlation.key`. Never log or export this.
    pub(crate) fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    pub fn token(&self, domain: CorrelationDomain, original: &str) -> CorrelationToken {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).expect("HMAC accepts any key length");
        mac.update(TOKEN_DOMAIN_PREFIX);
        mac.update(&[0]);
        mac.update(domain.tag());
        mac.update(&[0]);
        mac.update(original.as_bytes());
        let digest = mac.finalize().into_bytes();
        let mut token = [0u8; 32];
        token.copy_from_slice(&digest);
        CorrelationToken::from_bytes(token)
    }
}

/// Tokenization facade that is `None` while the key is unavailable. Call sites store an
/// optional token and keep working; they do not substitute the raw identifier.
#[derive(Clone, Default)]
pub struct Tokenizer {
    key: Option<CorrelationKey>,
}

impl std::fmt::Debug for Tokenizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.key {
            Some(_) => f.write_str("Tokenizer(available)"),
            None => f.write_str("Tokenizer(unavailable)"),
        }
    }
}

impl Tokenizer {
    pub fn available(key: CorrelationKey) -> Self {
        Self { key: Some(key) }
    }

    pub fn unavailable() -> Self {
        Self { key: None }
    }

    pub fn is_available(&self) -> bool {
        self.key.is_some()
    }

    /// Tokenize an existing identifier. `None` means correlation is unavailable; the
    /// caller records a null token plus a `correlation_unavailable` reason.
    pub fn token(&self, domain: CorrelationDomain, original: &str) -> Option<CorrelationToken> {
        self.key.as_ref().map(|key| key.token(domain, original))
    }

    /// Link two tokens that were produced from the same tokenizer. Only the layer that
    /// holds the real relationship may call this.
    pub fn token_pair(
        &self,
        left: (CorrelationDomain, &str),
        right: (CorrelationDomain, &str),
    ) -> Option<(CorrelationToken, CorrelationToken)> {
        let left = self.token(left.0, left.1)?;
        let right = self.token(right.0, right.1)?;
        Some((left, right))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_stable_per_key_domain_and_id() {
        let key = CorrelationKey::from_bytes([7u8; KEY_LEN]);
        let tokenizer = Tokenizer::available(key);
        let first = tokenizer
            .token(CorrelationDomain::ModelRequest, "req-1")
            .expect("token");
        let again = tokenizer
            .token(CorrelationDomain::ModelRequest, "req-1")
            .expect("token");
        assert_eq!(first, again);
        assert_eq!(first.to_hex().len(), 64);
    }

    #[test]
    fn domains_and_ids_do_not_collide() {
        let key = CorrelationKey::from_bytes([7u8; KEY_LEN]);
        let tokenizer = Tokenizer::available(key);
        let request = tokenizer
            .token(CorrelationDomain::ModelRequest, "shared-id")
            .expect("token");
        let attempt = tokenizer
            .token(CorrelationDomain::Attempt, "shared-id")
            .expect("token");
        let other = tokenizer
            .token(CorrelationDomain::ModelRequest, "other-id")
            .expect("token");
        assert_ne!(request, attempt);
        assert_ne!(request, other);
    }

    #[test]
    fn different_keys_produce_different_tokens() {
        let first = Tokenizer::available(CorrelationKey::from_bytes([1u8; KEY_LEN]));
        let second = Tokenizer::available(CorrelationKey::from_bytes([2u8; KEY_LEN]));
        let left = first.token(CorrelationDomain::Run, "run-9").expect("token");
        let right = second
            .token(CorrelationDomain::Run, "run-9")
            .expect("token");
        assert_ne!(left, right);
    }

    #[test]
    fn unavailable_tokenizer_never_falls_back_to_the_id() {
        let tokenizer = Tokenizer::unavailable();
        assert!(!tokenizer.is_available());
        assert!(
            tokenizer
                .token(CorrelationDomain::ControlRequest, "raw-request-id")
                .is_none()
        );
    }

    #[test]
    fn key_debug_is_redacted() {
        let key = CorrelationKey::from_bytes([9u8; KEY_LEN]);
        let rendered = format!("{key:?}");
        assert!(!rendered.contains("09"));
        assert!(rendered.contains("redacted"));
    }
}
