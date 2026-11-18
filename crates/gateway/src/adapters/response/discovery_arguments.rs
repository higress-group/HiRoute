//! Some native Responses providers stream client discovery arguments as JSON
//! text deltas but return an object in the completed item.
use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};
use serde_json::Value;

const MAX_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct DiscoveryArguments {
    bytes: Vec<u8>,
    charge: Option<Reservation>,
}

impl DiscoveryArguments {
    pub(super) fn append(&mut self, delta: &[u8], budget: &StreamBudget) -> Option<()> {
        let size = self.bytes.len().checked_add(delta.len())?;
        if size > MAX_BYTES {
            return None;
        }
        if size > self.bytes.capacity() {
            let capacity = size.next_power_of_two().max(64);
            let charge = budget.reserve(MemoryRole::SemanticState, capacity).ok()?;
            let mut bytes = Vec::with_capacity(capacity);
            bytes.extend_from_slice(&self.bytes);
            self.bytes = bytes;
            self.charge = Some(charge);
        }
        self.bytes.extend_from_slice(delta);
        Some(())
    }

    pub(super) fn digest(&self, budget: &StreamBudget) -> Option<[u8; 32]> {
        Self::object_digest(&self.bytes, budget)
    }

    pub(super) fn object_digest(bytes: &[u8], budget: &StreamBudget) -> Option<[u8; 32]> {
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let size = crate::content_ref::inline_document_retention_bytes(bytes)?;
        let _charge = budget.reserve(MemoryRole::SemanticState, size).ok()?;
        let value: Value = serde_json::from_slice(bytes).ok()?;
        Some(super::digest_item(value.as_object()?))
    }
}
