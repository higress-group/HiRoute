//! Exact Gateway v2 content-blob verification and streaming installation helpers.

pub(crate) mod completeness;
pub(crate) mod lifecycle;
pub(crate) mod projection;
pub(crate) mod storage;

use std::fmt;

use hiroute_domain::{CacheAffinityKey, CanonicalDigest, ContentBlobDigest, WorkspaceId};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroize;

type HmacSha256 = Hmac<Sha256>;
const CANONICALIZATION_VERSION: &[u8] = b"hiroute.model-request-ir/v1";

pub struct DigestAuthority {
    key: [u8; 32],
}

impl DigestAuthority {
    pub(crate) fn query_cursor_signature(&self, bytes: &[u8]) -> String {
        let mut mac = self.mac(b"observation-query-cursor/v2");
        update_field(&mut mac, bytes);
        hex(mac.finalize().into_bytes().as_slice())
    }

    pub(crate) fn verify_query_cursor_signature(&self, bytes: &[u8], signature: &str) -> bool {
        if signature.len() != 64 {
            return false;
        }
        let decoded: Option<Vec<u8>> = signature
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let digit = |b: u8| match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    _ => None,
                };
                Some(digit(pair[0])? * 16 + digit(pair[1])?)
            })
            .collect();
        let Some(decoded) = decoded else {
            return false;
        };
        let mut mac = self.mac(b"observation-query-cursor/v2");
        update_field(&mut mac, bytes);
        mac.verify_slice(&decoded).is_ok()
    }

    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    pub(crate) fn content_accumulator(&self, media_type: &str) -> ContentDigestAccumulator {
        let mut blob = self.mac(b"conversation-content-blob");
        update_field(&mut blob, CANONICALIZATION_VERSION);
        update_field(&mut blob, media_type.as_bytes());
        ContentDigestAccumulator {
            blob,
            byte_count: 0,
        }
    }

    pub fn content_blob_digest(&self, media_type: &str, bytes: &[u8]) -> ContentBlobDigest {
        let mut accumulator = self.content_accumulator(media_type);
        accumulator.update(bytes);
        accumulator.finish().0
    }

    /// Cache affinity remains a separate Product namespace and is never a transcript identity.
    pub fn cache_affinity_key(
        &self,
        workspace_id: &WorkspaceId,
        target_serialized_facts: &[u8],
    ) -> CacheAffinityKey {
        let mut mac = self.mac(b"hiroute/cache-affinity/v1");
        update_field(&mut mac, workspace_id.as_str().as_bytes());
        update_field(&mut mac, target_serialized_facts);
        CacheAffinityKey::parse(encode_hmac(mac.finalize().into_bytes().as_slice()))
            .expect("HMAC output has a valid cache-affinity representation")
    }

    /// Produces the durable idempotency comparison value for a bounded delegation request.
    /// The caller supplies canonical JSON bytes and retains the body separately in managed text;
    /// this keyed value prevents a low-entropy goal from becoming a raw prompt-hash index.
    pub fn delegation_request_digest(&self, canonical_request: &[u8]) -> CanonicalDigest {
        let mut mac = self.mac(b"hiroute/delegation-request/v1");
        update_field(&mut mac, canonical_request);
        // CanonicalDigest has a fixed `sha256:` wire representation. Hash the HMAC output so
        // the persisted record remains non-reversible and does not claim that request bytes
        // themselves were unhashed SHA-256 input.
        CanonicalDigest::of_bytes(mac.finalize().into_bytes().as_slice())
    }

    /// Keyed exact-title lookup material for the daemon-local Worker task index.
    pub fn worker_title_lookup_digest(
        &self,
        workspace: &WorkspaceId,
        normalized_title: &str,
    ) -> CanonicalDigest {
        let mut mac = self.mac(b"hiroute/worker-title-lookup/v1");
        update_field(&mut mac, workspace.as_str().as_bytes());
        update_field(&mut mac, normalized_title.as_bytes());
        CanonicalDigest::of_bytes(mac.finalize().into_bytes().as_slice())
    }

    /// Sign one bounded opaque Worker cursor purpose. The caller still validates the typed
    /// payload before accepting a cursor; purposes keep list and read tokens non-interchangeable.
    pub fn worker_cursor_signature(&self, purpose: &str, bytes: &[u8]) -> String {
        let mut mac = self.mac(b"hiroute/worker-cursor/v1");
        update_field(&mut mac, purpose.as_bytes());
        update_field(&mut mac, bytes);
        hex(mac.finalize().into_bytes().as_slice())
    }

    pub fn verify_worker_cursor_signature(
        &self,
        purpose: &str,
        bytes: &[u8],
        signature: &str,
    ) -> bool {
        if signature.len() != 64 {
            return false;
        }
        let decoded: Option<Vec<u8>> = signature
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let digit = |byte: u8| match byte {
                    b'0'..=b'9' => Some(byte - b'0'),
                    b'a'..=b'f' => Some(byte - b'a' + 10),
                    _ => None,
                };
                Some(digit(pair[0])? * 16 + digit(pair[1])?)
            })
            .collect();
        let Some(decoded) = decoded else {
            return false;
        };
        let mut mac = self.mac(b"hiroute/worker-cursor/v1");
        update_field(&mut mac, purpose.as_bytes());
        update_field(&mut mac, bytes);
        mac.verify_slice(&decoded).is_ok()
    }

    fn mac(&self, domain: &[u8]) -> HmacSha256 {
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("32-byte HMAC key is valid");
        update_field(&mut mac, domain);
        mac
    }
}

impl Clone for DigestAuthority {
    fn clone(&self) -> Self {
        Self { key: self.key }
    }
}

impl fmt::Debug for DigestAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DigestAuthority([redacted])")
    }
}

impl Drop for DigestAuthority {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

pub(crate) struct ContentDigestAccumulator {
    blob: HmacSha256,
    byte_count: u64,
}

impl ContentDigestAccumulator {
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        // Gateway uses update_stream here: chunks are one logically continuous byte string.
        self.blob.update(bytes);
        self.byte_count = self.byte_count.saturating_add(bytes.len() as u64);
    }

    pub(crate) fn finish(self) -> (ContentBlobDigest, u64) {
        let digest = format!("blob-{}", hex(self.blob.finalize().into_bytes().as_slice()));
        (
            ContentBlobDigest::parse(digest)
                .expect("HMAC output has a valid Gateway blob representation"),
            self.byte_count,
        )
    }
}

fn update_field(mac: &mut HmacSha256, bytes: &[u8]) {
    mac.update(&(bytes.len() as u64).to_be_bytes());
    mac.update(bytes);
}

fn encode_hmac(bytes: &[u8]) -> String {
    format!("hmac-sha256:{}", hex(bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_digest_is_chunk_boundary_independent_and_uses_gateway_namespace() {
        let authority = DigestAuthority::new([7; 32]);
        let direct = authority.content_blob_digest("text/plain", b"same");
        let mut streamed = authority.content_accumulator("text/plain");
        streamed.update(b"sa");
        streamed.update(b"me");
        assert_eq!(direct, streamed.finish().0);
        assert!(direct.as_str().starts_with("blob-"));
    }

    #[test]
    fn delegation_request_digest_is_keyed_and_domain_separated() {
        let first = DigestAuthority::new([7; 32]);
        let second = DigestAuthority::new([8; 32]);
        let request = br#"{\"goal\":\"low entropy\"}"#;

        let digest = first.delegation_request_digest(request);
        assert_eq!(digest, first.delegation_request_digest(request));
        assert_ne!(digest, second.delegation_request_digest(request));
        assert_ne!(digest, CanonicalDigest::of_bytes(request));
    }
}
