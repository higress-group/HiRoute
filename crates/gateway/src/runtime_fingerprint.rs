//! Process-local equality evidence. Never use this for authentication, stored
//! observation identities, recovery integrity, or other security boundaries.
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};
use md5::{Digest, Md5};

use crate::content_ref::ContentRef;
use crate::replay::{ReplayError, ReplayStore};

const MAX_CACHE_BYTES: usize = 256 * 1024;
const MAX_CACHE_ENTRIES: usize = 1024;
const ENTRY_BYTES: usize = 256;

/// Keep the existing private history-state carrier to avoid changing unrelated
/// identity/ticket types. Only the first 16 bytes contain MD5; the rest are zero.
/// This carrier is ephemeral, versioned by the hash domain, and never persisted.
#[derive(Clone)]
pub(crate) struct RuntimeHash(Md5);

impl RuntimeHash {
    pub(crate) fn new() -> Self {
        Self(Md5::new_with_prefix(b"hiroute-runtime-equality/md5-v1"))
    }

    pub(crate) fn new_with_prefix(bytes: impl AsRef<[u8]>) -> Self {
        let mut hash = Self::new();
        hash.update(bytes);
        hash
    }

    pub(crate) fn update(&mut self, bytes: impl AsRef<[u8]>) {
        self.0.update(bytes);
    }

    pub(crate) fn finalize(self) -> [u8; 32] {
        let mut output = [0; 32];
        output[..16].copy_from_slice(&self.0.finalize());
        output
    }

    pub(crate) fn digest(bytes: impl AsRef<[u8]>) -> [u8; 32] {
        Self::new_with_prefix(bytes).finalize()
    }
}

pub(crate) struct DigestWriter {
    hash: RuntimeHash,
    length: u64,
}

impl DigestWriter {
    pub(crate) fn new() -> Self {
        Self {
            hash: RuntimeHash::new(),
            length: 0,
        }
    }

    pub(crate) fn finish(self) -> (u64, [u8; 32]) {
        (self.length, self.hash.finalize())
    }
}

impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.length = self
            .length
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("fingerprint length overflow"))?;
        self.hash.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct ContentFingerprints(Mutex<Cache>);

#[derive(Default)]
struct Cache {
    references: BTreeMap<ContentRef, [u8; 32]>,
    inline: BTreeMap<Vec<u8>, [u8; 32]>,
    reservations: Vec<Reservation>,
    bytes: usize,
    #[cfg(test)]
    computations: usize,
}

impl ContentFingerprints {
    #[cfg(test)]
    pub(crate) fn computations(&self) -> usize {
        self.0.lock().unwrap().computations
    }

    pub(crate) fn reference(
        &self,
        replay: &ReplayStore,
        reference: &ContentRef,
    ) -> Result<[u8; 32], ReplayError> {
        if !replay.has_reference(reference) {
            return Err(ReplayError::UnknownContentRef);
        }
        let mut cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(digest) = cache.references.get(reference) {
            return Ok(*digest);
        }
        // Only sealed request-owned ranges reach this cache. The input is
        // immutable through Replay's API; a changed range has a different key.
        let mut reader = replay.reader(reference)?;
        let mut hash = RuntimeHash::new();
        let mut buffer = [0; 16 * 1024];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        reader.verify_terminal()?;
        let digest = hash.finalize();
        #[cfg(test)]
        {
            cache.computations += 1;
        }
        if cache.admit(replay.budget(), ENTRY_BYTES) {
            cache.references.insert(reference.clone(), digest);
        }
        Ok(digest)
    }

    pub(crate) fn inline(&self, budget: &StreamBudget, bytes: &[u8]) -> [u8; 32] {
        let mut cache = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(digest) = cache.inline.get(bytes) {
            return *digest;
        }
        let digest = RuntimeHash::digest(bytes);
        #[cfg(test)]
        {
            cache.computations += 1;
        }
        if cache.admit(budget, bytes.len().saturating_add(ENTRY_BYTES)) {
            cache.inline.insert(bytes.to_vec(), digest);
        }
        digest
    }
}

impl Cache {
    fn admit(&mut self, budget: &StreamBudget, bytes: usize) -> bool {
        if self.bytes.saturating_add(bytes) > MAX_CACHE_BYTES
            || self.references.len() + self.inline.len() >= MAX_CACHE_ENTRIES
        {
            return false;
        }
        let Ok(reservation) = budget.reserve(MemoryRole::ModelIrBacking, bytes) else {
            return false;
        };
        self.bytes += bytes;
        self.reservations.push(reservation);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_gateway_core::runtime::body::BudgetTree;

    #[test]
    fn inline_cache_measures_once_and_detects_changed_middle_bytes() {
        let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
            .unwrap()
            .stream(1024 * 1024)
            .unwrap();
        let before = budget.snapshot().unwrap().live;
        let cache = ContentFingerprints::default();
        let bytes = b"same prefix: middle content: same suffix";
        let first = cache.inline(&budget, bytes);
        for _ in 0..4 {
            assert_eq!(cache.inline(&budget, bytes), first);
        }
        assert_eq!(cache.0.lock().unwrap().computations, 1);
        let mut changed = bytes.to_vec();
        changed[bytes.len() / 2] ^= 1;
        assert_ne!(cache.inline(&budget, &changed), first);
        assert!(budget.snapshot().unwrap().live > before);
        drop(cache);
        assert_eq!(budget.snapshot().unwrap().live, before);
    }

    #[test]
    fn capacity_refusal_uses_complete_uncached_content_and_fragmentation_is_irrelevant() {
        let budget = BudgetTree::new(256, 256).unwrap().stream(256).unwrap();
        let cache = ContentFingerprints::default();
        let content = b"complete content including the middle";
        assert_eq!(
            cache.inline(&budget, content),
            cache.inline(&budget, content)
        );
        assert_eq!(cache.0.lock().unwrap().computations, 2);
        assert_eq!(budget.snapshot().unwrap().live, 0);
        let mut hash = RuntimeHash::new();
        for part in content.chunks(3) {
            hash.update(part);
        }
        assert_eq!(hash.finalize(), RuntimeHash::digest(content));
    }
}
