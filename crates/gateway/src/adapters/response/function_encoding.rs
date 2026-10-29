//! Function-argument encoding is declared by the selected dialect/producer, not guessed
//! from the readability of an argument. A final native snapshot cannot contradict a
//! declaration already delivered in this response.
use super::*;

impl DecoderCore {
    pub(super) fn observe_function_encoding(
        &mut self,
        index: u32,
        item: &Map<String, Value>,
        final_item: bool,
    ) -> Result<(), ProtocolAdapterError> {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            if item.contains_key("encrypted_function_args") {
                return Err(ModelIrError::UnsupportedField(
                    "encrypted_function_args on non-function item".into(),
                )
                .into());
            }
            return Ok(());
        }
        let mut value = super::super::responses_dialect::response_encoding(
            item,
            self.standard_responses_json,
            self.function_encoding_projection,
        )?;
        if !self.responses_client {
            if value.as_ref().is_some_and(|paths| !paths.is_empty()) {
                return Err(ModelIrError::UnsupportedField(
                    "encrypted function response has no portable encoding".into(),
                )
                .into());
            }
            // Standard JSON target protocols carry plaintext arguments intrinsically;
            // their replay has no Responses declaration field.
            value = None;
        }
        if let Some((previous, was_final)) = self.function_encodings.get(&index)
            && (*was_final || previous.is_some())
            && previous != &value
        {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "function argument encoding changed".into(),
            )
            .into());
        }
        if !self.function_encodings.contains_key(&index)
            || self.function_encodings[&index].0 != value
        {
            self.retention.add(
                32 + value
                    .as_ref()
                    .map_or(0, |paths| paths.iter().map(|p| 24 + p.len()).sum()),
            )?;
        }
        self.function_encodings.insert(index, (value, final_item));
        Ok(())
    }
}
