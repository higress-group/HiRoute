//! Whole-JSON responses grow against their memory owner, not a protocol size cap.
use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};

use super::{ModelIrError, ProtocolAdapterError};

/// Reservations grow geometrically with retained semantic data. There is no
/// independent protocol byte quota; allocation competes with the request.
#[derive(Clone, Debug)]
pub(super) struct Retention {
    budget: StreamBudget,
    bytes: usize,
    charged: usize,
    charges: Vec<std::sync::Arc<Reservation>>,
}

impl Retention {
    pub(super) fn new(budget: StreamBudget) -> Self {
        Self {
            budget,
            bytes: 0,
            charged: 0,
            charges: Vec::new(),
        }
    }

    pub(super) fn add(&mut self, bytes: usize) -> Result<(), ProtocolAdapterError> {
        let total = self
            .bytes
            .checked_add(bytes)
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        // Accumulator growth and a staged terminal reconciliation may coexist.
        let required = total
            .checked_mul(4)
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        if required > self.charged {
            let target = required.checked_next_power_of_two().unwrap_or(required);
            let charge = self
                .budget
                .reserve(MemoryRole::SemanticState, target - self.charged)
                .map_err(|_| ModelIrError::BufferLimit(required))?;
            self.charges.push(std::sync::Arc::new(charge));
            self.charged = target;
        }
        self.bytes = total;
        Ok(())
    }
}

pub(super) fn standalone_budget() -> StreamBudget {
    const MEMORY: usize = 256 * 1024 * 1024;
    hiroute_gateway_core::runtime::body::BudgetTree::new(MEMORY, MEMORY)
        .expect("fixed valid budget")
        .stream(MEMORY)
        .expect("fixed valid stream")
}

#[derive(Default)]
pub(super) struct BodyBuffer {
    bytes: Vec<u8>,
    charge: Option<Reservation>,
}

impl BodyBuffer {
    pub(super) fn append(
        &mut self,
        bytes: &[u8],
        budget: &StreamBudget,
    ) -> Result<(), ProtocolAdapterError> {
        let needed = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        if needed > self.bytes.capacity() {
            let capacity = needed.max(self.bytes.capacity().saturating_mul(2));
            let charge = budget
                .reserve(MemoryRole::ResponsePrefix, capacity)
                .map_err(|_| ModelIrError::BufferLimit(capacity))?;
            // Keep the old reservation alive until its allocation is dropped.
            let mut replacement = Vec::with_capacity(capacity);
            replacement.extend_from_slice(&self.bytes);
            self.bytes = replacement;
            self.charge = Some(charge);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn len(&self) -> usize {
        self.bytes.len()
    }
}
