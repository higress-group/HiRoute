//! Provenance observation for opaque state in native passthrough responses.
use super::super::super::continuation::record_provider_state_at_acceptance;
use super::*;

impl ProjectionState {
    pub(super) fn observe_reasoning_state(
        &mut self,
        index: u32,
        value: &Value,
    ) -> Result<(), ProtocolAdapterError> {
        let digest: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&value)
                .map_err(|error| ProtocolAdapterError::Serialization(error.to_string()))?,
        )
        .into();
        if self.reasoning_state.get(&index) == Some(&digest) {
            return Ok(());
        }
        if !self.reasoning_state.contains_key(&index) {
            let charge = self
                .budget
                .reserve(MemoryRole::SemanticState, 128)
                .map_err(|_| ModelIrError::BufferLimit(128))?;
            self.retained.push(charge);
        }
        if self.authority.records_state() {
            record_provider_state(value, &self.owner)?;
        }
        self.reasoning_state.insert(index, digest);
        Ok(())
    }

    pub(super) fn observe_messages_state(
        &mut self,
        object: &Map<String, Value>,
        raw_event: &[u8],
    ) -> Result<(), ProtocolAdapterError> {
        // Off-path observation must never issue continuation authority.
        if !self.authority.records_state() {
            return Ok(());
        }
        match object.get("type").and_then(Value::as_str) {
            Some("content_block_start") if object["content_block"]["type"] == "thinking" => {
                let index = u32_field(object, "index")?;
                if self.messages_signatures.contains_key(&index) {
                    return Err(ModelIrError::InvalidField("thinking.index").into());
                }
                let charge = self
                    .budget
                    .reserve(MemoryRole::SemanticState, 128)
                    .map_err(|_| ModelIrError::BufferLimit(128))?;
                self.retained.push(charge);
                self.messages_signatures.insert(index, String::new());
                if let Some(value) = object["content_block"]
                    .get("signature")
                    .and_then(Value::as_str)
                {
                    self.append_messages_signature(index, value)?;
                }
            }
            Some("content_block_delta") if object["delta"]["type"] == "signature_delta" => {
                let index = u32_field(object, "index")?;
                let value = object["delta"]["signature"]
                    .as_str()
                    .ok_or(ModelIrError::InvalidField("signature"))?;
                self.append_messages_signature(index, value)?;
            }
            Some("content_block_stop") => {
                let index = u32_field(object, "index")?;
                if let Some(signature) = self.messages_signatures.remove(&index)
                    && !signature.is_empty()
                {
                    record_provider_state_at_acceptance(&signature, &self.owner, raw_event)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn append_messages_signature(
        &mut self,
        index: u32,
        delta: &str,
    ) -> Result<(), ProtocolAdapterError> {
        let signature = self
            .messages_signatures
            .get_mut(&index)
            .ok_or(ModelIrError::InvalidField("signature.index"))?;
        if delta.is_empty() {
            return Ok(());
        }
        let bytes = delta
            .len()
            .checked_add(2 * std::mem::size_of::<Reservation>())
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        let charge = self
            .budget
            .reserve(MemoryRole::SemanticState, bytes)
            .map_err(|_| ModelIrError::BufferLimit(bytes))?;
        signature.reserve_exact(delta.len());
        signature.push_str(delta);
        self.retained.push(charge);
        Ok(())
    }
}
