use sha2::{Digest, Sha256};

const SHA256_BLOCK_BYTES: usize = 64;

#[derive(Clone)]
pub(super) struct WorkspaceHmac {
    inner: Sha256,
    outer_pad: [u8; SHA256_BLOCK_BYTES],
}

impl WorkspaceHmac {
    pub(super) fn new(key: &[u8; 32], domain: &[u8]) -> Self {
        let mut key_block = [0_u8; SHA256_BLOCK_BYTES];
        key_block[..key.len()].copy_from_slice(key);
        let mut inner_pad = [0x36_u8; SHA256_BLOCK_BYTES];
        let mut outer_pad = [0x5c_u8; SHA256_BLOCK_BYTES];
        for ((inner, outer), key) in inner_pad
            .iter_mut()
            .zip(outer_pad.iter_mut())
            .zip(key_block)
        {
            *inner ^= key;
            *outer ^= key;
        }
        let mut inner = Sha256::new();
        inner.update(inner_pad);
        inner.update((domain.len() as u64).to_be_bytes());
        inner.update(domain);
        Self { inner, outer_pad }
    }

    pub(super) fn update(&mut self, bytes: &[u8]) {
        self.inner.update((bytes.len() as u64).to_be_bytes());
        self.inner.update(bytes);
    }

    /// Adds one fragment of a logically continuous byte string. Callers must
    /// domain-separate and length-prefix preceding structured fields before
    /// streaming canonical bytes through this method.
    pub(super) fn update_stream(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }

    pub(super) fn finish(self) -> String {
        let inner = self.inner.finalize();
        let mut outer = Sha256::new();
        outer.update(self.outer_pad);
        outer.update(inner);
        format!("sha256:{:x}", outer.finalize())
    }
}

pub(super) fn stable_id(prefix: &str, key: &[u8; 32], domain: &[u8], parts: &[&[u8]]) -> String {
    let mut hmac = WorkspaceHmac::new(key, domain);
    for part in parts {
        hmac.update(part);
    }
    format!("{prefix}-{}", hmac.finish().trim_start_matches("sha256:"))
}

pub(super) fn random_key() -> Option<[u8; 32]> {
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key).ok().map(|()| key)
}
